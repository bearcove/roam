use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use facet::Facet;
use facet_core::{PtrConst, Shape};
use facet_reflect::Peek;
use roam_types::{
    Backing, Conduit, ConduitRx, ConduitTx, ConduitTxPermit, Link, LinkRx, LinkTx, LinkTxPermit,
    MsgFamily, RpcPlan, SelfRef, WriteSlot,
};

use crate::replay_buffer::{PacketAck, PacketSeq, ReplayBuffer};

// ---------------------------------------------------------------------------
// Frame — internal to StableConduit
// ---------------------------------------------------------------------------

/// Opaque session identifier for reliability-layer resume.
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
struct ResumeKey(Vec<u8>);

/// Client's opening handshake.
#[derive(Facet, Debug, Clone)]
struct ClientHello {
    resume_key: Option<ResumeKey>,
    last_received: Option<PacketSeq>,
}

/// Server's handshake response.
#[derive(Facet, Debug, Clone)]
struct ServerHello {
    resume_key: ResumeKey,
    last_received: Option<PacketSeq>,
    // r[impl stable.reconnect.failure]
    /// If true, the server did not recognize the resume_key and the client
    /// must treat this as a session loss.
    rejected: bool,
}

/// Sequenced data frame. All post-handshake traffic is `Frame<T>`.
/// Serialized in a single postcard pass — the seq/ack fields are just
/// the first fields of the serialized output.
// r[impl stable.framing]
// r[impl stable.framing.encoding]
#[derive(Facet, Debug, Clone)]
struct Frame<T> {
    seq: PacketSeq,
    // r[impl stable.ack]
    ack: Option<PacketAck>,
    item: T,
}

// ---------------------------------------------------------------------------
// Attachment / LinkSource
// ---------------------------------------------------------------------------

struct Attachment<L> {
    link: L,
    client_hello: Option<ClientHello>,
}

// r[impl stable.link-source]
pub trait LinkSource: Send + 'static {
    type Link: Link;

    #[allow(async_fn_in_trait)]
    async fn next_link(&mut self) -> std::io::Result<Attachment<Self::Link>>;
}

// ---------------------------------------------------------------------------
// StableConduit
// ---------------------------------------------------------------------------

// r[impl stable]
pub struct StableConduit<F: MsgFamily, LS: LinkSource> {
    shared: Arc<Shared<LS>>,
    frame_shape: &'static Shape,
    _phantom: PhantomData<fn(F) -> F>,
}

struct Shared<LS: LinkSource> {
    inner: Mutex<Inner<LS>>,
    reconnecting: AtomicBool,
    reconnected: tokio::sync::Notify,
}

struct Inner<LS: LinkSource> {
    source: Option<LS>,
    /// Incremented every time the link is replaced. Used to detect whether
    /// another task has already reconnected while we were waiting.
    link_generation: u64,
    tx: Option<<LS::Link as Link>::Tx>,
    rx: Option<<LS::Link as Link>::Rx>,
    resume_key: Option<ResumeKey>,
    // r[impl stable.seq]
    next_send_seq: PacketSeq,
    last_received: Option<PacketSeq>,
    // r[impl stable.replay-buffer]
    /// Encoded item bytes buffered for replay on reconnect.
    replay: ReplayBuffer,
}

impl<F: MsgFamily, LS: LinkSource> StableConduit<F, LS> {
    pub async fn new(mut source: LS, plan: &'static RpcPlan) -> Result<Self, StableConduitError> {
        let shape = F::shape();
        assert!(
            plan.shape == shape,
            "RpcPlan shape mismatch: plan is for {}, expected {}",
            plan.shape,
            shape,
        );

        let attachment = source.next_link().await.map_err(StableConduitError::Io)?;
        let (link_tx, mut link_rx) = attachment.link.split();

        let (resume_key, _peer_last_received) =
            handshake::<LS::Link>(&link_tx, &mut link_rx, attachment.client_hello, None, None)
                .await?;

        let inner = Inner {
            source: Some(source),
            link_generation: 0,
            tx: Some(link_tx),
            rx: Some(link_rx),
            resume_key: Some(resume_key),
            next_send_seq: PacketSeq(0),
            last_received: None,
            replay: ReplayBuffer::new(),
        };

        Ok(Self {
            shared: Arc::new(Shared {
                inner: Mutex::new(inner),
                reconnecting: AtomicBool::new(false),
                reconnected: tokio::sync::Notify::new(),
            }),
            frame_shape: Frame::<F::Msg<'static>>::SHAPE,
            _phantom: PhantomData,
        })
    }
}

// ---------------------------------------------------------------------------
// Reconnect
// ---------------------------------------------------------------------------

impl<LS: LinkSource> Shared<LS> {
    fn lock_inner(&self) -> Result<MutexGuard<'_, Inner<LS>>, StableConduitError> {
        self.inner
            .lock()
            .map_err(|_| StableConduitError::Setup("stable conduit mutex poisoned".into()))
    }

    async fn ensure_reconnected(&self, generation: u64) -> Result<(), StableConduitError> {
        loop {
            {
                let inner = self.lock_inner()?;
                if inner.link_generation != generation {
                    return Ok(());
                }
            }

            if self
                .reconnecting
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let result = self.reconnect_once(generation).await;
                self.reconnecting.store(false, Ordering::Release);
                self.reconnected.notify_waiters();
                return result;
            }

            self.reconnected.notified().await;
        }
    }

    /// Obtain a new link from the source, re-handshake, and replay any
    /// buffered items the peer missed.
    // r[impl stable.reconnect]
    // r[impl stable.reconnect.client-replay]
    // r[impl stable.reconnect.server-replay]
    // r[impl stable.replay-buffer.order]
    async fn reconnect_once(&self, generation: u64) -> Result<(), StableConduitError> {
        let (mut source, resume_key, last_received, replay_frames) = {
            let mut inner = self.lock_inner()?;
            if inner.link_generation != generation {
                return Ok(());
            }
            let source = inner
                .source
                .take()
                .ok_or_else(|| StableConduitError::Setup("link source unavailable".into()))?;
            let replay_frames = inner
                .replay
                .iter()
                .map(|(seq, bytes)| (*seq, bytes.clone()))
                .collect::<Vec<_>>();
            (
                source,
                inner.resume_key.clone(),
                inner.last_received,
                replay_frames,
            )
        };

        let reconnect_result = async {
            let attachment = source.next_link().await.map_err(StableConduitError::Io)?;
            let (new_tx, mut new_rx) = attachment.link.split();

            let (new_resume_key, peer_last_received) = handshake::<LS::Link>(
                &new_tx,
                &mut new_rx,
                attachment.client_hello,
                resume_key,
                last_received,
            )
            .await?;

            // Replay frames the peer hasn't received yet, in original order.
            // Frame bytes include the original seq/ack — stale acks are
            // harmless since the peer ignores acks older than what it has seen.
            for (seq, frame_bytes) in replay_frames {
                if peer_last_received.is_some_and(|last| seq <= last) {
                    continue;
                }
                let permit = new_tx.reserve().await.map_err(StableConduitError::Io)?;
                let mut slot = permit
                    .alloc(frame_bytes.len())
                    .map_err(StableConduitError::Io)?;
                slot.as_mut_slice().copy_from_slice(&frame_bytes);
                slot.commit();
            }

            Ok::<_, StableConduitError>((new_tx, new_rx, new_resume_key))
        }
        .await;

        let mut inner = self.lock_inner()?;
        inner.source = Some(source);

        if inner.link_generation != generation {
            return Ok(());
        }

        let (new_tx, new_rx, new_resume_key) = reconnect_result?;

        inner.link_generation = inner.link_generation.wrapping_add(1);
        inner.tx = Some(new_tx);
        inner.rx = Some(new_rx);
        inner.resume_key = Some(new_resume_key);

        Ok(())
    }
}

/// Perform the handshake on a fresh link.
///
/// Returns `(our_resume_key, peer_last_received)`:
///   - `our_resume_key`: the key to use for the next reconnect attempt
///   - `peer_last_received`: the highest seq the peer has already seen,
///     used to decide which replay-buffer entries to re-send
// r[impl stable.handshake]
// r[impl stable.handshake.client-hello]
// r[impl stable.handshake.server-hello]
async fn handshake<L: Link>(
    tx: &L::Tx,
    rx: &mut L::Rx,
    client_hello: Option<ClientHello>,
    resume_key: Option<ResumeKey>,
    last_received: Option<PacketSeq>,
) -> Result<(ResumeKey, Option<PacketSeq>), StableConduitError> {
    match client_hello {
        None => {
            // r[impl stable.reconnect]
            let hello = ClientHello {
                resume_key,
                last_received,
            };
            send_msg(tx, &hello).await?;
            let server_hello: ServerHello = recv_msg(rx).await?;
            // r[impl stable.reconnect.failure]
            if server_hello.rejected {
                return Err(StableConduitError::SessionLost);
            }
            Ok((server_hello.resume_key, server_hello.last_received))
        }
        Some(client_hello) => {
            // r[impl stable.resume-key]
            let key = ResumeKey(fresh_key());
            let hello = ServerHello {
                resume_key: key.clone(),
                last_received,
                rejected: false,
            };
            send_msg(tx, &hello).await?;
            Ok((key, client_hello.last_received))
        }
    }
}

async fn send_msg<LTx: LinkTx, M: Facet<'static>>(
    tx: &LTx,
    msg: &M,
) -> Result<(), StableConduitError> {
    let bytes = facet_postcard::to_vec(msg).map_err(StableConduitError::Encode)?;
    let permit = tx.reserve().await.map_err(StableConduitError::Io)?;
    let mut slot = permit.alloc(bytes.len()).map_err(StableConduitError::Io)?;
    slot.as_mut_slice().copy_from_slice(&bytes);
    slot.commit();
    Ok(())
}

async fn recv_msg<LRx: LinkRx, M: Facet<'static>>(rx: &mut LRx) -> Result<M, StableConduitError> {
    let backing = rx
        .recv()
        .await
        .map_err(|_| StableConduitError::LinkDead)?
        .ok_or(StableConduitError::LinkDead)?;
    let bytes: &[u8] = match &backing {
        Backing::Boxed(b) => b,
    };
    facet_postcard::from_slice(bytes).map_err(StableConduitError::Decode)
}

fn fresh_key() -> Vec<u8> {
    // TODO: use a proper CSPRNG
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let mut buf = [0u8; 16];
    buf[..8].copy_from_slice(&t.as_nanos().to_le_bytes()[..8]);
    buf.to_vec()
}

// ---------------------------------------------------------------------------
// Conduit impl
// ---------------------------------------------------------------------------

impl<F: MsgFamily, LS: LinkSource> Conduit for StableConduit<F, LS>
where
    <LS::Link as Link>::Tx: Clone + Send + 'static,
    <LS::Link as Link>::Rx: Send + 'static,
    LS: Send + 'static,
{
    type Msg<'a> = F::Msg<'a>;
    type Tx = StableConduitTx<F, LS>;
    type Rx = StableConduitRx<F, LS>;

    fn split(self) -> (Self::Tx, Self::Rx) {
        (
            StableConduitTx {
                shared: Arc::clone(&self.shared),
                _phantom: PhantomData,
            },
            StableConduitRx {
                shared: Arc::clone(&self.shared),
                frame_shape: self.frame_shape,
                _phantom: PhantomData,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Tx
// ---------------------------------------------------------------------------

pub struct StableConduitTx<F: MsgFamily, LS: LinkSource> {
    shared: Arc<Shared<LS>>,
    _phantom: PhantomData<fn(F)>,
}

impl<F: MsgFamily, LS: LinkSource> ConduitTx for StableConduitTx<F, LS>
where
    <LS::Link as Link>::Tx: Clone + Send + 'static,
    <LS::Link as Link>::Rx: Send + 'static,
    LS: Send + 'static,
{
    type Msg<'a> = F::Msg<'a>;
    type Permit<'a>
        = StableConduitPermit<F, LS>
    where
        Self: 'a;

    async fn reserve(&self) -> std::io::Result<Self::Permit<'_>> {
        loop {
            let (tx, generation) = {
                let inner = self
                    .shared
                    .lock_inner()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
                (inner.tx.clone(), inner.link_generation)
            };

            let tx = match tx {
                Some(tx) => tx,
                None => {
                    self.shared
                        .ensure_reconnected(generation)
                        .await
                        .map_err(|e| {
                            std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
                        })?;
                    continue;
                }
            };

            match tx.reserve().await {
                Ok(link_permit) => {
                    return Ok(StableConduitPermit {
                        shared: Arc::clone(&self.shared),
                        link_permit,
                        generation,
                        _phantom: PhantomData,
                    });
                }
                Err(_) => {
                    self.shared
                        .ensure_reconnected(generation)
                        .await
                        .map_err(|e| {
                            std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
                        })?;
                }
            }
        }
    }

    async fn close(self) -> std::io::Result<()> {
        let tx = {
            let mut inner = self
                .shared
                .lock_inner()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
            inner.tx.take()
        };
        if let Some(tx) = tx {
            tx.close().await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Permit
// ---------------------------------------------------------------------------

pub struct StableConduitPermit<F: MsgFamily, LS: LinkSource> {
    shared: Arc<Shared<LS>>,
    link_permit: <<LS::Link as Link>::Tx as LinkTx>::Permit,
    generation: u64,
    _phantom: PhantomData<fn(F)>,
}

impl<F: MsgFamily, LS: LinkSource> ConduitTxPermit for StableConduitPermit<F, LS> {
    type Msg<'a> = F::Msg<'a>;
    type Error = StableConduitError;

    // r[impl zerocopy.framing.single-pass]
    // r[impl zerocopy.framing.no-double-serialize]
    // r[impl zerocopy.scatter.write]
    // r[impl zerocopy.scatter.replay]
    fn send(self, item: F::Msg<'_>) -> Result<(), StableConduitError> {
        let StableConduitPermit {
            shared,
            link_permit,
            generation,
            _phantom: _,
        } = self;

        let (seq, ack) = {
            let mut inner = shared.lock_inner()?;
            if inner.link_generation != generation {
                return Err(StableConduitError::LinkDead);
            }
            let seq = inner.next_send_seq;
            inner.next_send_seq = PacketSeq(seq.0.wrapping_add(1));
            let ack = inner
                .last_received
                .map(|max_delivered| PacketAck { max_delivered });
            (seq, ack)
        };

        let frame = Frame { seq, ack, item };

        // SAFETY: The shape matches `frame`'s concrete type for all lifetimes.
        // `Frame<F::Msg<'a>>` has a lifetime-independent shape by `MsgFamily` contract.
        #[allow(unsafe_code)]
        let peek = unsafe {
            Peek::unchecked_new(
                PtrConst::new((&raw const frame).cast::<u8>()),
                Frame::<F::Msg<'static>>::SHAPE,
            )
        };
        let plan =
            facet_postcard::peek_to_scatter_plan(peek).map_err(StableConduitError::Encode)?;

        let mut slot = link_permit
            .alloc(plan.total_size())
            .map_err(StableConduitError::Io)?;
        let slot_bytes = slot.as_mut_slice();
        plan.write_into(slot_bytes)
            .map_err(StableConduitError::Encode)?;

        // Keep an owned copy for replay after reconnect.
        shared.lock_inner()?.replay.push(seq, slot_bytes.to_vec());
        slot.commit();

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Rx
// ---------------------------------------------------------------------------

pub struct StableConduitRx<F: MsgFamily, LS: LinkSource> {
    shared: Arc<Shared<LS>>,
    frame_shape: &'static Shape,
    _phantom: PhantomData<fn() -> F>,
}

impl<F: MsgFamily, LS: LinkSource> ConduitRx for StableConduitRx<F, LS>
where
    <LS::Link as Link>::Tx: Send + 'static,
    <LS::Link as Link>::Rx: Send + 'static,
    LS: Send + 'static,
{
    type Msg<'a> = F::Msg<'a>;
    type Error = StableConduitError;

    async fn recv(&mut self) -> Result<Option<SelfRef<F::Msg<'static>>>, Self::Error> {
        loop {
            // Phase 1: take current Rx out of shared state, then await without locks held.
            let (mut rx, generation) = {
                let mut inner = self.shared.lock_inner()?;
                let generation = inner.link_generation;
                let rx = match inner.rx.take() {
                    Some(rx) => rx,
                    None => {
                        drop(inner);
                        self.shared.ensure_reconnected(generation).await?;
                        continue;
                    }
                };
                (rx, generation)
            };

            // Any link termination — graceful EOF or error — triggers reconnect.
            // The session ends only when the LinkSource itself fails (no more
            // links available), which surfaces as Err.
            let recv_result = rx.recv().await;

            // Put Rx back only if we're still on the same generation and no newer
            // Rx has been installed by reconnect.
            {
                let mut inner = self.shared.lock_inner()?;
                if inner.link_generation == generation && inner.rx.is_none() {
                    inner.rx = Some(rx);
                }
            }

            let backing = match recv_result {
                Ok(Some(b)) => b,
                Ok(None) | Err(_) => {
                    // r[impl stable.reconnect]
                    self.shared.ensure_reconnected(generation).await?;
                    continue;
                }
            };

            // Phase 2: deserialize the frame.
            let frame_shape = self.frame_shape;
            let frame: SelfRef<Frame<F::Msg<'static>>> =
                SelfRef::try_new(backing, |bytes| deserialize_frame(frame_shape, bytes))?;

            // Phase 3: update shared state; skip duplicates.
            // r[impl stable.seq.monotonic]
            // r[impl stable.ack.trim]
            let is_dup = {
                let mut inner = self.shared.lock_inner()?;

                if let Some(ack) = frame.ack {
                    inner.replay.trim(ack);
                }

                let dup = inner.last_received.is_some_and(|prev| frame.seq <= prev);
                if !dup {
                    inner.last_received = Some(frame.seq);
                }
                dup
            };

            if is_dup {
                continue;
            }

            return Ok(Some(frame.map(|f| f.item)));
        }
    }
}

fn deserialize_frame<T: 'static>(
    shape: &'static Shape,
    bytes: &[u8],
) -> Result<Frame<T>, StableConduitError> {
    use facet_format::{FormatDeserializer, MetaSource};
    use facet_postcard::PostcardParser;
    use facet_reflect::Partial;

    let mut value = std::mem::MaybeUninit::<Frame<T>>::uninit();
    let ptr = facet_core::PtrUninit::new(value.as_mut_ptr().cast::<u8>());

    // SAFETY: ptr points to valid, aligned, properly-sized memory for Frame<T>.
    // shape is Frame<T>::SHAPE, set at StableConduit construction time.
    #[allow(unsafe_code)]
    let partial: Partial<'_, false> = unsafe { Partial::from_raw_with_shape(ptr, shape) }
        .map_err(|e| StableConduitError::Decode(e.into()))?;

    let mut parser = PostcardParser::new(bytes);
    let mut deserializer = FormatDeserializer::new_owned(&mut parser);
    let partial = deserializer
        .deserialize_into(partial, MetaSource::FromEvents)
        .map_err(StableConduitError::Decode)?;

    partial
        .finish_in_place()
        .map_err(|e| StableConduitError::Decode(e.into()))?;

    #[allow(unsafe_code)]
    Ok(unsafe { value.assume_init() })
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum StableConduitError {
    Encode(facet_postcard::SerializeError),
    Decode(facet_format::DeserializeError),
    Io(std::io::Error),
    LinkDead,
    Setup(String),
    /// The server rejected our resume_key; the session is permanently lost.
    // r[impl stable.reconnect.failure]
    SessionLost,
}

impl std::fmt::Display for StableConduitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode(e) => write!(f, "encode error: {e}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::LinkDead => write!(f, "link dead"),
            Self::Setup(s) => write!(f, "setup error: {s}"),
            Self::SessionLost => write!(f, "session lost: server rejected resume key"),
        }
    }
}

impl std::error::Error for StableConduitError {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::OnceLock;

    use roam_types::{
        Conduit, ConduitRx, ConduitTx, ConduitTxPermit, LinkRx, LinkTx, MsgFamily, RpcPlan,
    };

    use crate::{MemoryLink, memory_link_pair};

    use super::*;

    struct StringFamily;

    impl MsgFamily for StringFamily {
        type Msg<'a> = String;

        fn shape() -> &'static facet_core::Shape {
            String::SHAPE
        }
    }

    fn string_plan() -> &'static RpcPlan {
        static PLAN: OnceLock<RpcPlan> = OnceLock::new();
        PLAN.get_or_init(|| RpcPlan::for_type::<String, (), ()>())
    }

    // A LinkSource backed by a queue of pre-created MemoryLinks.
    struct QueuedLinkSource {
        links: VecDeque<(MemoryLink, Option<ClientHello>)>,
    }

    impl LinkSource for QueuedLinkSource {
        type Link = MemoryLink;

        async fn next_link(&mut self) -> std::io::Result<Attachment<MemoryLink>> {
            let (link, client_hello) = self.links.pop_front().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "no more links")
            })?;
            Ok(Attachment { link, client_hello })
        }
    }

    // Encode and send a frame directly onto a LinkTx.
    async fn send_frame<LTx: LinkTx>(tx: &LTx, seq: u32, ack: Option<u32>, item: &str) {
        let frame = Frame {
            seq: PacketSeq(seq),
            ack: ack.map(|n| PacketAck {
                max_delivered: PacketSeq(n),
            }),
            item: item.to_string(),
        };
        let frame_bytes = facet_postcard::to_vec(&frame).unwrap();

        let permit = tx.reserve().await.unwrap();
        let mut slot = permit.alloc(frame_bytes.len()).unwrap();
        slot.as_mut_slice().copy_from_slice(&frame_bytes);
        slot.commit();
    }

    // Decode a raw frame payload into (seq, ack_max, item).
    fn decode_frame(bytes: &[u8]) -> (u32, Option<u32>, String) {
        let frame: Frame<String> = super::deserialize_frame(Frame::<String>::SHAPE, bytes).unwrap();
        (
            frame.seq.0,
            frame.ack.map(|a| a.max_delivered.0),
            frame.item,
        )
    }

    // Receive one raw payload from a LinkRx.
    async fn recv_raw<LRx: LinkRx>(rx: &mut LRx) -> Vec<u8> {
        let backing = rx.recv().await.unwrap().unwrap();
        match backing {
            roam_types::Backing::Boxed(b) => b.to_vec(),
        }
    }

    // ---------------------------------------------------------------------------
    // Basic StableConduit tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn stable_send_recv_single() {
        let (c, s) = memory_link_pair(16);

        let source = QueuedLinkSource {
            links: VecDeque::from([(c, None)]),
        };

        // Server-side: complete handshake then send a frame.
        let server = tokio::spawn(async move {
            let (s_tx, mut s_rx) = s.split();
            let _hello: ClientHello = recv_msg(&mut s_rx).await.unwrap();
            send_msg(
                &s_tx,
                &ServerHello {
                    resume_key: ResumeKey(b"key".to_vec()),
                    last_received: None,
                    rejected: false,
                },
            )
            .await
            .unwrap();

            // Receive one frame from client.
            let raw = recv_raw(&mut s_rx).await;
            let (seq, _, item) = decode_frame(&raw);
            (seq, item)
        });

        let client = StableConduit::<StringFamily, _>::new(source, string_plan())
            .await
            .unwrap();
        let (client_tx, _client_rx) = client.split();

        let permit = client_tx.reserve().await.unwrap();
        permit.send("hello".to_string()).unwrap();

        let (seq, item) = server.await.unwrap();
        assert_eq!(seq, 0);
        assert_eq!(item, "hello");
    }

    // ---------------------------------------------------------------------------
    // Reconnect tests
    // ---------------------------------------------------------------------------

    /// Client sends A and B. Server acks A. Link dies.
    /// On reconnect, server reports last_received = Some(0) (saw A).
    /// Client replays B (seq 1). Server receives it on the new link.
    #[tokio::test]
    async fn reconnect_replays_unacked_frames() {
        let (c1, s1) = memory_link_pair(32);
        let (c2, s2) = memory_link_pair(32);

        // Link 1: server receives A and B, acks A, then drops.
        let server1 = tokio::spawn(async move {
            let (s1_tx, mut s1_rx) = s1.split();

            let _hello: ClientHello = recv_msg(&mut s1_rx).await.unwrap();
            send_msg(
                &s1_tx,
                &ServerHello {
                    resume_key: ResumeKey(b"resume-key-for-test".to_vec()),
                    last_received: None,
                    rejected: false,
                },
            )
            .await
            .unwrap();

            // Receive A (seq 0)
            let raw = recv_raw(&mut s1_rx).await;
            let (seq_a, _, item_a) = decode_frame(&raw);
            assert_eq!(seq_a, 0);
            assert_eq!(item_a, "alpha");

            // Receive B (seq 1)
            let raw = recv_raw(&mut s1_rx).await;
            let (seq_b, _, item_b) = decode_frame(&raw);
            assert_eq!(seq_b, 1);
            assert_eq!(item_b, "beta");

            // Send ack for seq 0 (server has received A but NOT B as far as client knows)
            // The client will trim replay buffer entry for seq 0 after receiving this.
            send_frame(&s1_tx, 0, Some(0), "ack-for-alpha").await;

            // Drop — link dies, triggering reconnect on client.
        });

        // Link 2: server handles reconnect, replays, receives replayed B.
        let server2 = tokio::spawn(async move {
            let (s2_tx, mut s2_rx) = s2.split();

            let hello: ClientHello = recv_msg(&mut s2_rx).await.unwrap();
            // Client should present a resume key.
            assert!(hello.resume_key.is_some());
            // Client received one frame from server (seq 0), so last_received = Some(0).
            assert_eq!(hello.last_received, Some(PacketSeq(0)));

            // Server says it received up to seq 0 from the client (it saw A but not B).
            send_msg(
                &s2_tx,
                &ServerHello {
                    resume_key: ResumeKey(b"resume-key-2".to_vec()),
                    last_received: Some(PacketSeq(0)),
                    rejected: false,
                },
            )
            .await
            .unwrap();

            // Client should replay B (seq 1) automatically.
            let raw = recv_raw(&mut s2_rx).await;
            let (seq, _, item) = decode_frame(&raw);
            assert_eq!(seq, 1);
            assert_eq!(item, "beta");

            // New message after reconnect (seq 2).
            let raw = recv_raw(&mut s2_rx).await;
            let (seq, _, item) = decode_frame(&raw);
            assert_eq!(seq, 2);
            assert_eq!(item, "gamma");
        });

        // Client side.
        let source = QueuedLinkSource {
            links: VecDeque::from([(c1, None), (c2, None)]),
        };
        let client = StableConduit::<StringFamily, _>::new(source, string_plan())
            .await
            .unwrap();
        let (client_tx, mut client_rx) = client.split();

        // Send A and B.
        client_tx
            .reserve()
            .await
            .unwrap()
            .send("alpha".to_string())
            .unwrap();
        client_tx
            .reserve()
            .await
            .unwrap()
            .send("beta".to_string())
            .unwrap();

        // Receive the ack frame from server1. This trims seq 0 from replay buffer,
        // leaving only seq 1 (beta) buffered.
        let msg = client_rx.recv().await.unwrap().unwrap();
        assert_eq!(&*msg, "ack-for-alpha");

        // server1 drops — recv triggers reconnect transparently.
        // After reconnect, client replays beta, then we send gamma.
        client_tx
            .reserve()
            .await
            .unwrap()
            .send("gamma".to_string())
            .unwrap();

        server1.await.unwrap();
        server2.await.unwrap();
    }

    /// On reconnect, server says it has seen everything. Client sends nothing extra.
    #[tokio::test]
    async fn reconnect_no_replay_when_all_acked() {
        let (c1, s1) = memory_link_pair(32);
        let (c2, s2) = memory_link_pair(32);

        let server1 = tokio::spawn(async move {
            let (s1_tx, mut s1_rx) = s1.split();
            let _: ClientHello = recv_msg(&mut s1_rx).await.unwrap();
            send_msg(
                &s1_tx,
                &ServerHello {
                    resume_key: ResumeKey(b"key1".to_vec()),
                    last_received: None,
                    rejected: false,
                },
            )
            .await
            .unwrap();

            // Receive A and B.
            recv_raw(&mut s1_rx).await;
            recv_raw(&mut s1_rx).await;

            // Ack both.
            send_frame(&s1_tx, 0, Some(1), "ack-both").await;
            // Drop.
        });

        let server2 = tokio::spawn(async move {
            let (s2_tx, mut s2_rx) = s2.split();
            let hello: ClientHello = recv_msg(&mut s2_rx).await.unwrap();
            assert!(hello.resume_key.is_some());

            // Server has seen everything (up to seq 1).
            send_msg(
                &s2_tx,
                &ServerHello {
                    resume_key: ResumeKey(b"key2".to_vec()),
                    last_received: Some(PacketSeq(1)),
                    rejected: false,
                },
            )
            .await
            .unwrap();

            // Only the new message (seq 2) should arrive — no replay.
            let raw = recv_raw(&mut s2_rx).await;
            let (seq, _, item) = decode_frame(&raw);
            assert_eq!(seq, 2);
            assert_eq!(item, "gamma");
        });

        let source = QueuedLinkSource {
            links: VecDeque::from([(c1, None), (c2, None)]),
        };
        let client = StableConduit::<StringFamily, _>::new(source, string_plan())
            .await
            .unwrap();
        let (client_tx, mut client_rx) = client.split();

        client_tx
            .reserve()
            .await
            .unwrap()
            .send("alpha".to_string())
            .unwrap();
        client_tx
            .reserve()
            .await
            .unwrap()
            .send("beta".to_string())
            .unwrap();

        let msg = client_rx.recv().await.unwrap().unwrap();
        assert_eq!(&*msg, "ack-both");

        // Reconnect happens transparently here.
        client_tx
            .reserve()
            .await
            .unwrap()
            .send("gamma".to_string())
            .unwrap();

        server1.await.unwrap();
        server2.await.unwrap();
    }

    /// After reconnect, duplicate frames (seq <= last_received) are silently dropped.
    #[tokio::test]
    async fn duplicate_frames_are_skipped() {
        let (c, s) = memory_link_pair(32);

        let source = QueuedLinkSource {
            links: VecDeque::from([(c, None)]),
        };

        let server = tokio::spawn(async move {
            let (s_tx, mut s_rx) = s.split();
            let _: ClientHello = recv_msg(&mut s_rx).await.unwrap();
            send_msg(
                &s_tx,
                &ServerHello {
                    resume_key: ResumeKey(b"k".to_vec()),
                    last_received: None,
                    rejected: false,
                },
            )
            .await
            .unwrap();

            // Send seq 0, then a duplicate seq 0, then seq 1.
            send_frame(&s_tx, 0, None, "first").await;
            send_frame(&s_tx, 0, None, "duplicate-first").await;
            send_frame(&s_tx, 1, None, "second").await;
        });

        let client = StableConduit::<StringFamily, _>::new(source, string_plan())
            .await
            .unwrap();
        let (_client_tx, mut client_rx) = client.split();

        let a = client_rx.recv().await.unwrap().unwrap();
        assert_eq!(&*a, "first");

        // The duplicate seq 0 is silently dropped, so next is "second".
        let b = client_rx.recv().await.unwrap().unwrap();
        assert_eq!(&*b, "second");

        server.await.unwrap();
    }

    /// When the server rejects the resume_key, recv() returns SessionLost.
    // r[verify stable.reconnect.failure]
    #[tokio::test]
    async fn reconnect_failure_surfaces_session_lost() {
        let (c1, s1) = memory_link_pair(32);
        let (c2, s2) = memory_link_pair(32);

        // Server 1: accept initial connection, send ack, then drop.
        let server1 = tokio::spawn(async move {
            let (s1_tx, mut s1_rx) = s1.split();
            let _: ClientHello = recv_msg(&mut s1_rx).await.unwrap();
            send_msg(
                &s1_tx,
                &ServerHello {
                    resume_key: ResumeKey(b"known-key".to_vec()),
                    last_received: None,
                    rejected: false,
                },
            )
            .await
            .unwrap();
            recv_raw(&mut s1_rx).await;
            // Drop — triggers reconnect on client.
        });

        // Server 2: receives reconnect attempt but rejects the resume_key.
        let server2 = tokio::spawn(async move {
            let (s2_tx, mut s2_rx) = s2.split();
            let hello: ClientHello = recv_msg(&mut s2_rx).await.unwrap();
            assert!(hello.resume_key.is_some());
            // Reject the resume attempt.
            send_msg(
                &s2_tx,
                &ServerHello {
                    resume_key: ResumeKey(vec![]),
                    last_received: None,
                    rejected: true,
                },
            )
            .await
            .unwrap();
        });

        let source = QueuedLinkSource {
            links: VecDeque::from([(c1, None), (c2, None)]),
        };
        let client = StableConduit::<StringFamily, _>::new(source, string_plan())
            .await
            .unwrap();
        let (client_tx, mut client_rx) = client.split();

        client_tx
            .reserve()
            .await
            .unwrap()
            .send("hello".to_string())
            .unwrap();

        // server1 drops → reconnect → server2 rejects → SessionLost
        match client_rx.recv().await {
            Err(StableConduitError::SessionLost) => {}
            other => panic!("expected SessionLost, got: {:?}", other.map(|_| ())),
        }

        server1.await.unwrap();
        server2.await.unwrap();
    }
}
