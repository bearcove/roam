use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use roam_types::{
    Attachment, Codec, Link, LinkRx, LinkSource, LinkTx, LinkTxPermit, Packet, PacketAck,
    PacketSeq, ReliableHello, SelfRef,
};

/// Shared mutable state between the Tx and Rx halves of a [`ReliableLink`].
struct Shared<T> {
    /// Unacked outbound packets, held for replay on reconnect.
    /// Front = oldest (lowest seq), back = newest.
    /// Tx pushes, Rx trims (when processing inbound acks).
    replay_buffer: VecDeque<ReplayEntry<T>>,

    /// Highest contiguous seq delivered to the upper layer from inbound.
    /// Rx writes, Tx reads (to piggyback acks on outbound packets).
    rx_delivered: Option<PacketSeq>,
}

/// An outbound packet buffered for potential replay after reconnect.
struct ReplayEntry<T> {
    seq: PacketSeq,
    item: T,
}

/// Reliable, ordered delivery over an unreliable [`Link`].
///
/// Wraps a `Link<Packet<T>, C>` and implements `Link<T, C>`.
/// Strips the packet framing — upper layers see only `T`.
///
/// On link failure, obtains a replacement via [`LinkSource::next_link`],
/// completes the Hello exchange, and replays unacknowledged packets.
///
/// Generic over `T` — does not know what `T` is. Only understands
/// [`Packet`]: assigns sequence numbers, processes acks, buffers for
/// replay, deduplicates inbound.
pub struct ReliableLink<T: Send + 'static, C: Codec, S: LinkSource<T, C>> {
    inner_tx: <S::Link as Link<Packet<T>, C>>::Tx,
    inner_rx: <S::Link as Link<Packet<T>, C>>::Rx,
    source: S,
    shared: Arc<Mutex<Shared<T>>>,
    next_seq: Arc<AtomicU32>,
    resume_key: Option<roam_types::ResumeKey>,
}

impl<T: Send + 'static, C: Codec, S: LinkSource<T, C>> ReliableLink<T, C, S> {
    /// Create a new `ReliableLink` from an initial attachment and a source
    /// for future replacement links.
    ///
    /// If `attachment.peer_hello` is `Some`, the peer's Hello was already
    /// consumed (server side) and we only send our Hello response.
    /// If `None`, we do the full Hello exchange (client side).
    pub async fn new(
        source: S,
        attachment: Attachment<S::Link>,
        resume_key: Option<roam_types::ResumeKey>,
    ) -> std::io::Result<Self> {
        let (inner_tx, inner_rx) = attachment.link.split();

        let shared = Arc::new(Mutex::new(Shared {
            replay_buffer: VecDeque::new(),
            rx_delivered: None,
        }));

        let mut this = Self {
            inner_tx,
            inner_rx,
            source,
            shared,
            next_seq: Arc::new(AtomicU32::new(0)),
            resume_key,
        };

        this.complete_hello(attachment.peer_hello).await?;

        Ok(this)
    }

    /// Complete the Hello exchange on the current link.
    ///
    /// - If `peer_hello` is Some: server side. Peer already said Hello,
    ///   we just send our response.
    /// - If `peer_hello` is None: client side. We send Hello first,
    ///   then read the peer's response.
    async fn complete_hello(&mut self, peer_hello: Option<ReliableHello>) -> std::io::Result<()> {
        let our_hello = ReliableHello {
            resume_key: self.resume_key.clone(),
            last_received: self
                .shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .rx_delivered,
        };

        match peer_hello {
            Some(_peer) => {
                // Server side: peer already sent Hello. Send our response.
                self.send_hello(our_hello).await?;
            }
            None => {
                // Client side: send Hello, then read peer's response.
                self.send_hello(our_hello).await?;
                self.read_hello().await?;
            }
        }

        Ok(())
    }

    /// Send a Hello packet on the inner link.
    async fn send_hello(&self, hello: ReliableHello) -> std::io::Result<()> {
        let permit = self.inner_tx.reserve().await?;
        permit
            .send(Packet::Hello(hello))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Read a Hello packet from the inner link.
    async fn read_hello(&mut self) -> std::io::Result<ReliableHello> {
        let packet = self
            .inner_rx
            .recv()
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::ConnectionReset, e.to_string()))?
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed before Hello",
                )
            })?;

        match packet.map(|p| match p {
            Packet::Hello(h) => Ok(h),
            Packet::Data { .. } => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expected Hello, got Data",
            )),
        }) {
            // SelfRef::map gives us SelfRef<Result<...>>, deref to get the Result
            ref sr => match &**sr {
                Ok(h) => Ok(h.clone()),
                Err(e) => Err(std::io::Error::new(e.kind(), e.to_string())),
            },
        }
    }
}

impl<T: Send + Clone + 'static, C: Codec, S: LinkSource<T, C>> Link<T, C>
    for ReliableLink<T, C, S>
{
    type Tx = ReliableLinkTx<T, C, S>;
    type Rx = ReliableLinkRx<T, C, S>;

    fn split(self) -> (Self::Tx, Self::Rx) {
        let tx = ReliableLinkTx {
            inner: self.inner_tx,
            shared: Arc::clone(&self.shared),
            next_seq: Arc::clone(&self.next_seq),
        };
        let rx = ReliableLinkRx {
            inner: self.inner_rx,
            shared: self.shared,
            expected_seq: PacketSeq(0),
        };
        (tx, rx)
    }
}

// ---------------------------------------------------------------------------
// Tx half
// ---------------------------------------------------------------------------

/// Sending half of a [`ReliableLink`].
pub struct ReliableLinkTx<T: Send + 'static, C: Codec, S: LinkSource<T, C>> {
    inner: <S::Link as Link<Packet<T>, C>>::Tx,
    shared: Arc<Mutex<Shared<T>>>,
    next_seq: Arc<AtomicU32>,
}

impl<T: Send + Clone + 'static, C: Codec, S: LinkSource<T, C>> LinkTx<T, C>
    for ReliableLinkTx<T, C, S>
where
    <S::Link as Link<Packet<T>, C>>::Tx: Send,
{
    type Permit<'a>
        = ReliableLinkPermit<'a, T, C, S>
    where
        Self: 'a;

    async fn reserve(&self) -> std::io::Result<Self::Permit<'_>> {
        let inner_permit = self.inner.reserve().await?;
        Ok(ReliableLinkPermit {
            inner: inner_permit,
            tx: self,
        })
    }

    async fn close(self) -> std::io::Result<()> {
        self.inner.close().await
    }
}

// ---------------------------------------------------------------------------
// Permit
// ---------------------------------------------------------------------------

/// Permit for sending one item through a [`ReliableLinkTx`].
pub struct ReliableLinkPermit<'a, T: Send + 'static, C: Codec, S: LinkSource<T, C>> {
    inner: <<S::Link as Link<Packet<T>, C>>::Tx as LinkTx<Packet<T>, C>>::Permit<'a>,
    tx: &'a ReliableLinkTx<T, C, S>,
}

impl<T: Send + Clone + 'static, C: Codec, S: LinkSource<T, C>> LinkTxPermit<T, C>
    for ReliableLinkPermit<'_, T, C, S>
{
    fn send(self, item: T) -> Result<(), C::EncodeError> {
        let seq = PacketSeq(self.tx.next_seq.fetch_add(1, Ordering::Relaxed));

        let ack = self
            .tx
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rx_delivered
            .map(|s| PacketAck { max_delivered: s });

        let packet = Packet::Data {
            seq,
            ack,
            item: item.clone(),
        };

        // Buffer for replay before sending — if send fails, the item is
        // still in the buffer for replay after reconnect.
        self.tx
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replay_buffer
            .push_back(ReplayEntry { seq, item });

        self.inner.send(packet)
    }
}

// ---------------------------------------------------------------------------
// Rx half
// ---------------------------------------------------------------------------

/// Receiving half of a [`ReliableLink`].
pub struct ReliableLinkRx<T: Send + 'static, C: Codec, S: LinkSource<T, C>> {
    inner: <S::Link as Link<Packet<T>, C>>::Rx,
    shared: Arc<Mutex<Shared<T>>>,
    /// Next expected inbound sequence number (for dedup and ordering).
    expected_seq: PacketSeq,
}

impl<T: Send + 'static, C: Codec, S: LinkSource<T, C>> LinkRx<T, C> for ReliableLinkRx<T, C, S>
where
    <S::Link as Link<Packet<T>, C>>::Rx: Send,
{
    type Error =
        ReliableLinkError<<<S::Link as Link<Packet<T>, C>>::Rx as LinkRx<Packet<T>, C>>::Error>;

    async fn recv(&mut self) -> Result<Option<SelfRef<T>>, Self::Error> {
        loop {
            let packet = match self
                .inner
                .recv()
                .await
                .map_err(ReliableLinkError::Transport)?
            {
                Some(p) => p,
                None => return Ok(None),
            };

            match &*packet {
                Packet::Hello(_) => {
                    // Hello after initial handshake = protocol violation, skip.
                    continue;
                }
                Packet::Data { seq, ack, .. } => {
                    // Process inbound ack: trim replay buffer.
                    if let Some(ack) = ack {
                        let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
                        while let Some(front) = shared.replay_buffer.front() {
                            if front.seq.0 <= ack.max_delivered.0 {
                                shared.replay_buffer.pop_front();
                            } else {
                                break;
                            }
                        }
                    }

                    let seq = *seq;

                    if seq.0 < self.expected_seq.0 {
                        // Duplicate (from replay overlap after reconnect). Skip.
                        continue;
                    }

                    if seq.0 > self.expected_seq.0 {
                        // Gap on an ordered transport — protocol error.
                        return Err(ReliableLinkError::SequenceGap {
                            expected: self.expected_seq,
                            got: seq,
                        });
                    }

                    // seq == expected_seq: deliver.
                    self.expected_seq = PacketSeq(seq.0.wrapping_add(1));
                    self.shared
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .rx_delivered = Some(seq);

                    // Project SelfRef<Packet<T>> → SelfRef<T>.
                    let item = packet.map(|p| match p {
                        Packet::Data { item, .. } => item,
                        Packet::Hello(_) => unreachable!("already matched Data above"),
                    });
                    return Ok(Some(item));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors from [`ReliableLinkRx::recv`].
pub enum ReliableLinkError<E> {
    /// The underlying transport returned an error.
    Transport(E),
    /// Received a sequence number ahead of expected (gap on ordered transport).
    SequenceGap { expected: PacketSeq, got: PacketSeq },
}

impl<E: std::fmt::Debug> std::fmt::Debug for ReliableLinkError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => f.debug_tuple("Transport").field(e).finish(),
            Self::SequenceGap { expected, got } => f
                .debug_struct("SequenceGap")
                .field("expected", expected)
                .field("got", got)
                .finish(),
        }
    }
}

impl<E: std::fmt::Display> std::fmt::Display for ReliableLinkError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "transport error: {e}"),
            Self::SequenceGap { expected, got } => {
                write!(f, "sequence gap: expected {}, got {}", expected.0, got.0)
            }
        }
    }
}

impl<E: std::error::Error> std::error::Error for ReliableLinkError<E> {}
