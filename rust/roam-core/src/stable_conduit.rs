use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::Arc;

use facet::Facet;
use facet_reflect::TypePlanCore;
use roam_types::{
    Backing, Conduit, ConduitRx, ConduitTx, ConduitTxPermit, Link, LinkRx, LinkTx, LinkTxPermit,
    RpcPlan, SelfRef, WriteSlot,
};

// ---------------------------------------------------------------------------
// Frame — internal to StableConduit
// ---------------------------------------------------------------------------

#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
#[facet(transparent)]
struct PacketSeq(u32);

#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PacketAck {
    max_delivered: PacketSeq,
}

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
}

/// Frame header — seq + ack, serialized separately so we can prepend to
/// already-encoded item bytes without needing an owned T.
#[derive(Facet, Debug, Clone, Copy)]
struct FrameHeader {
    seq: PacketSeq,
    ack: Option<PacketAck>,
}

/// Sequenced data frame. All post-handshake traffic is `Frame<T>`.
#[derive(Facet, Debug, Clone)]
struct Frame<T> {
    seq: PacketSeq,
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

pub trait LinkSource: Send + 'static {
    type Link: Link;

    async fn next_link(&mut self) -> std::io::Result<Attachment<Self::Link>>;
}

// ---------------------------------------------------------------------------
// StableConduit
// ---------------------------------------------------------------------------

// r[impl conduit.stable]
pub struct StableConduit<T: 'static, LS: LinkSource> {
    inner: Arc<tokio::sync::Mutex<Inner<LS>>>,
    frame_plan: Arc<TypePlanCore>,
    _phantom: PhantomData<fn() -> T>,
}

struct Inner<LS: LinkSource> {
    source: LS,
    tx: Option<<LS::Link as Link>::Tx>,
    rx: <LS::Link as Link>::Rx,
    resume_key: Option<ResumeKey>,
    next_send_seq: PacketSeq,
    last_received: Option<PacketSeq>,
    /// (seq, encoded item bytes) — replayed on reconnect with fresh header.
    replay: VecDeque<(PacketSeq, Vec<u8>)>,
}

impl<T: Facet<'static> + 'static, LS: LinkSource> StableConduit<T, LS> {
    pub async fn new(mut source: LS, plan: &'static RpcPlan) -> Result<Self, StableConduitError> {
        assert!(
            plan.shape == T::SHAPE,
            "RpcPlan shape mismatch: plan is for {}, expected {}",
            plan.shape,
            T::SHAPE,
        );

        let frame_plan = facet_reflect::TypePlan::<Frame<T>>::build()
            .map_err(|e| StableConduitError::Setup(e.to_string()))?
            .core();

        let attachment = source.next_link().await.map_err(StableConduitError::Io)?;
        let (link_tx, mut link_rx) = attachment.link.split();

        let (resume_key, last_received) =
            handshake::<LS::Link>(&link_tx, &mut link_rx, attachment.client_hello).await?;

        let inner = Inner {
            source,
            tx: Some(link_tx),
            rx: link_rx,
            resume_key: Some(resume_key),
            next_send_seq: PacketSeq(0),
            last_received,
            replay: VecDeque::new(),
        };

        Ok(Self {
            inner: Arc::new(tokio::sync::Mutex::new(inner)),
            frame_plan,
            _phantom: PhantomData,
        })
    }
}

async fn handshake<L: Link>(
    tx: &L::Tx,
    rx: &mut L::Rx,
    client_hello: Option<ClientHello>,
) -> Result<(ResumeKey, Option<PacketSeq>), StableConduitError> {
    match client_hello {
        None => {
            let hello = ClientHello {
                resume_key: None,
                last_received: None,
            };
            send_msg(tx, &hello).await?;
            let server_hello: ServerHello = recv_msg(rx).await?;
            Ok((server_hello.resume_key, server_hello.last_received))
        }
        Some(client_hello) => {
            let key = ResumeKey(fresh_key());
            let hello = ServerHello {
                resume_key: key.clone(),
                last_received: None,
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
// Conduit<T> impl
// ---------------------------------------------------------------------------

impl<T: Facet<'static> + 'static, LS: LinkSource> Conduit<T> for StableConduit<T, LS>
where
    <LS::Link as Link>::Tx: Clone + Send + 'static,
    <LS::Link as Link>::Rx: Send + 'static,
    LS: Send + 'static,
{
    type Tx = StableConduitTx<T, LS>;
    type Rx = StableConduitRx<T, LS>;

    fn split(self) -> (Self::Tx, Self::Rx) {
        (
            StableConduitTx {
                inner: Arc::clone(&self.inner),
                _phantom: PhantomData,
            },
            StableConduitRx {
                inner: Arc::clone(&self.inner),
                frame_plan: self.frame_plan,
                _phantom: PhantomData,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Tx
// ---------------------------------------------------------------------------

pub struct StableConduitTx<T: 'static, LS: LinkSource> {
    inner: Arc<tokio::sync::Mutex<Inner<LS>>>,
    _phantom: PhantomData<fn() -> T>,
}

impl<T: Facet<'static> + 'static, LS: LinkSource> ConduitTx<T> for StableConduitTx<T, LS>
where
    <LS::Link as Link>::Tx: Clone + Send + 'static,
    <LS::Link as Link>::Rx: Send + 'static,
    LS: Send + 'static,
{
    type Permit<'a>
        = StableConduitPermit<'a, T, LS>
    where
        Self: 'a;

    async fn reserve(&self) -> std::io::Result<Self::Permit<'_>> {
        let tx = {
            let inner = self.inner.lock().await;
            inner
                .tx
                .as_ref()
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stable conduit closed")
                })?
                .clone()
        };

        let link_permit = tx.reserve().await?;

        let guard = self.inner.lock().await;
        Ok(StableConduitPermit {
            guard,
            link_permit,
            _phantom: PhantomData,
        })
    }

    async fn close(self) -> std::io::Result<()> {
        let mut inner = self.inner.lock().await;
        if let Some(tx) = inner.tx.take() {
            tx.close().await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Permit
// ---------------------------------------------------------------------------

pub struct StableConduitPermit<'a, T: 'static, LS: LinkSource> {
    guard: tokio::sync::MutexGuard<'a, Inner<LS>>,
    link_permit: <<LS::Link as Link>::Tx as LinkTx>::Permit,
    _phantom: PhantomData<fn() -> T>,
}

impl<T: Facet<'static> + 'static, LS: LinkSource> ConduitTxPermit<T>
    for StableConduitPermit<'_, T, LS>
{
    type Error = StableConduitError;

    fn send(mut self, item: &T) -> Result<(), StableConduitError> {
        let inner = &mut *self.guard;

        let seq = inner.next_send_seq;
        inner.next_send_seq = PacketSeq(seq.0.wrapping_add(1));

        let ack = inner
            .last_received
            .map(|max_delivered| PacketAck { max_delivered });

        // Encode the item for the replay buffer.
        let item_bytes = facet_postcard::to_vec(item).map_err(StableConduitError::Encode)?;
        inner.replay.push_back((seq, item_bytes.clone()));

        // Wire encoding = header bytes + item bytes (matches Frame<T> postcard layout).
        let header_bytes = facet_postcard::to_vec(&FrameHeader { seq, ack })
            .map_err(StableConduitError::Encode)?;

        let mut frame_bytes = header_bytes;
        frame_bytes.extend_from_slice(&item_bytes);

        let mut slot = self
            .link_permit
            .alloc(frame_bytes.len())
            .map_err(StableConduitError::Io)?;
        slot.as_mut_slice().copy_from_slice(&frame_bytes);
        slot.commit();

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Rx
// ---------------------------------------------------------------------------

pub struct StableConduitRx<T: 'static, LS: LinkSource> {
    inner: Arc<tokio::sync::Mutex<Inner<LS>>>,
    frame_plan: Arc<TypePlanCore>,
    _phantom: PhantomData<fn() -> T>,
}

impl<T: Facet<'static> + 'static, LS: LinkSource> ConduitRx<T> for StableConduitRx<T, LS>
where
    <LS::Link as Link>::Tx: Send + 'static,
    <LS::Link as Link>::Rx: Send + 'static,
    LS: Send + 'static,
{
    type Error = StableConduitError;

    async fn recv(&mut self) -> Result<Option<SelfRef<T>>, Self::Error> {
        let backing = {
            let mut inner = self.inner.lock().await;
            inner
                .rx
                .recv()
                .await
                .map_err(|_| StableConduitError::LinkDead)?
        };

        let backing = match backing {
            Some(b) => b,
            None => return Ok(None),
        };

        let frame_plan = Arc::clone(&self.frame_plan);
        let frame_self_ref: SelfRef<Frame<T>> =
            SelfRef::try_new(backing, |bytes| deserialize_frame(&frame_plan, bytes))?;

        {
            let mut inner = self.inner.lock().await;

            if let Some(ack) = frame_self_ref.ack {
                while inner
                    .replay
                    .front()
                    .is_some_and(|(s, _)| *s <= ack.max_delivered)
                {
                    inner.replay.pop_front();
                }
            }

            match inner.last_received {
                None => inner.last_received = Some(frame_self_ref.seq),
                Some(prev) if frame_self_ref.seq > prev => {
                    inner.last_received = Some(frame_self_ref.seq)
                }
                _ => {}
            }
        }

        Ok(Some(frame_self_ref.map(|f| f.item)))
    }
}

fn deserialize_frame<T: 'static>(
    plan: &Arc<TypePlanCore>,
    bytes: &[u8],
) -> Result<Frame<T>, StableConduitError> {
    use facet_format::{FormatDeserializer, MetaSource};
    use facet_postcard::PostcardParser;
    use facet_reflect::Partial;

    let mut value = std::mem::MaybeUninit::<Frame<T>>::uninit();
    let ptr = facet_core::PtrUninit::new(value.as_mut_ptr().cast::<u8>());
    let root_id = plan.root_id();

    #[allow(unsafe_code)]
    let partial: Partial<'_, false> = unsafe { Partial::from_raw(ptr, Arc::clone(plan), root_id) }
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
}

impl std::fmt::Display for StableConduitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode(e) => write!(f, "encode error: {e}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::LinkDead => write!(f, "link dead"),
            Self::Setup(s) => write!(f, "setup error: {s}"),
        }
    }
}

impl std::error::Error for StableConduitError {}
