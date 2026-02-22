//! New connectivity layer: Link (raw bytes) + Conduit (typed).
//!
//! ## Layering
//!
//! ```text
//! ┌───────────────────────────────────────────┐
//! │  Session                                  │
//! │  knows Message, generic over Conduit      │
//! ├───────────────────────────────────────────┤
//! │  Conduit (trait)                          │
//! │  typed send/recv, codec is internal       │
//! │  ├── BareConduit: Link + codec            │
//! │  └── StableConduit: Link + codec          │
//! │      + seq/ack/replay (bytes in buffer)   │
//! ├───────────────────────────────────────────┤
//! │  Link (trait)                             │
//! │  raw bytes, transport provides buffers    │
//! │  ├── TcpLink                              │
//! │  ├── WebSocketLink                        │
//! │  └── ShmLink (bipbuffer/varslot)          │
//! └───────────────────────────────────────────┘
//! ```
//!
//! ## Key differences from connectivity.rs
//!
//! - **Link has no T or C generics** — it moves raw bytes.
//! - **Link provides write buffers** — `alloc(len)` returns a `WriteSlot`
//!   backed by the transport's own memory (bipbuffer for SHM, write buf for
//!   TCP). Caller encodes directly into the slot. One copy, not two.
//! - **Conduit owns serialization** — codec is an implementation detail,
//!   not a trait parameter. Session sees `impl Conduit`, never `C: Codec`.
//! - **StableConduit replay buffer stores bytes** — no `T: Clone` needed.
//! - **Both layers are splittable and use permits.**

#![allow(unsafe_code, async_fn_in_trait)]

use facet::Facet;

use crate::{Backing, SelfRef};

// ---------------------------------------------------------------------------
// Link — raw bytes transport
// ---------------------------------------------------------------------------

/// Bidirectional raw-bytes transport.
///
/// TCP, WebSocket, SHM all implement this. No knowledge of what's being
/// sent — just bytes in, bytes out. The transport provides write buffers
/// so callers can encode directly into the destination (zero-copy for SHM).
pub trait Link {
    type Tx: LinkTx;
    type Rx: LinkRx;

    fn split(self) -> (Self::Tx, Self::Rx);
}

/// Sending half of a [`Link`].
///
/// Uses a two-phase write: `alloc(len)` returns a [`WriteSlot`] backed by
/// the transport's own buffer (bipbuffer slot, kernel write buffer, etc.),
/// then the caller fills it and calls [`WriteSlot::commit`].
///
/// `alloc` is the backpressure point — it awaits until the transport has
/// room for `len` bytes.
pub trait LinkTx: Send + 'static {
    type Slot<'a>: WriteSlot
    where
        Self: 'a;

    /// Allocate a writable buffer of exactly `len` bytes.
    ///
    /// Backpressure: blocks until the transport can accommodate `len` bytes.
    /// For SHM: allocates from bipbuffer or varslot.
    /// For TCP: reserves space in the write buffer.
    ///
    /// Dropping the returned slot without committing discards it and
    /// releases the space.
    async fn alloc(&self, len: usize) -> std::io::Result<Self::Slot<'_>>;

    /// Graceful close of the outbound direction.
    async fn close(self) -> std::io::Result<()>
    where
        Self: Sized;
}

/// A writable slot in the transport's output buffer.
///
/// Obtained from [`LinkTx::alloc`]. The caller writes encoded bytes into
/// [`as_mut_slice`](WriteSlot::as_mut_slice), then calls
/// [`commit`](WriteSlot::commit) to make them visible to the receiver.
///
/// Dropping without commit = discard (no bytes sent, space reclaimed).
pub trait WriteSlot {
    /// The writable buffer, exactly the size requested in `alloc`.
    fn as_mut_slice(&mut self) -> &mut [u8];

    /// Commit the written bytes. After this, the receiver can see them.
    /// Sync — the bytes are already in the transport's buffer.
    fn commit(self);
}

/// Receiving half of a [`Link`].
///
/// Yields [`Backing`] values: the raw bytes plus their ownership handle.
/// The transport handles framing (length-prefix, WebSocket frames, etc.)
/// and returns exactly one message's bytes per `recv` call.
///
/// For SHM: the Backing might be a VarSlot reference.
/// For TCP: the Backing is a heap-allocated buffer.
pub trait LinkRx: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Receive the next message's raw bytes.
    ///
    /// Returns `Ok(None)` when the peer has closed the connection.
    async fn recv(&mut self) -> Result<Option<Backing>, Self::Error>;
}

/// A [`Link`] assembled from pre-split Tx and Rx halves.
pub struct SplitLink<Tx, Rx> {
    pub tx: Tx,
    pub rx: Rx,
}

impl<Tx: LinkTx, Rx: LinkRx> Link for SplitLink<Tx, Rx> {
    type Tx = Tx;
    type Rx = Rx;

    fn split(self) -> (Tx, Rx) {
        (self.tx, self.rx)
    }
}

// ---------------------------------------------------------------------------
// Conduit — typed, serialization-aware
// ---------------------------------------------------------------------------

/// Bidirectional typed transport. Wraps a [`Link`] and owns serialization.
///
/// Session is generic over this trait. The codec is an implementation detail
/// — it doesn't appear in the trait signature.
///
/// Two implementations:
/// - [`BareConduit`]: Link + codec. If the link dies, it's dead.
/// - [`StableConduit`]: Link + codec + seq/ack/replay. Handles reconnect
///   transparently. Replay buffer stores encoded bytes (no `T: Clone`).
pub trait Conduit {
    type Tx: ConduitTx;
    type Rx: ConduitRx;

    fn split(self) -> (Self::Tx, Self::Rx);
}

/// Sending half of a [`Conduit`].
///
/// Permit-based: `reserve()` is the backpressure point, `permit.send()`
/// serializes and writes.
pub trait ConduitTx: Send + 'static {
    type Permit<'a>: ConduitTxPermit
    where
        Self: 'a;

    /// Reserve capacity for one outbound message.
    ///
    /// Backpressure lives here — this may block waiting for:
    /// - StableConduit: replay buffer capacity (bounded outstanding)
    /// - Flow control from the peer
    ///
    /// Dropping the permit without sending releases the reservation.
    async fn reserve(&self) -> std::io::Result<Self::Permit<'_>>;

    /// Graceful close of the outbound direction.
    async fn close(self) -> std::io::Result<()>
    where
        Self: Sized;
}

/// Permit for sending exactly one message through a [`ConduitTx`].
///
/// `send` is async because it allocates from the underlying Link (which
/// may block for buffer space). The Conduit's `reserve()` handles
/// logical backpressure (outstanding messages), while the Link's `alloc`
/// handles physical backpressure (buffer bytes).
pub trait ConduitTxPermit {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Serialize `item` and send it through the link.
    ///
    /// Flow:
    /// 1. Create a `Peek` from `item`
    /// 2. `encode_scatter(peek)` → parts + total_size
    /// 3. `link_tx.alloc(total_size)` → write slot
    /// 4. Copy parts into slot
    /// 5. Commit (+ buffer encoded bytes for replay in StableConduit)
    ///
    /// Accepts `T: Facet<'a>` — the value may contain borrowed data (e.g.
    /// `&str` fields in RPC call arguments). The borrow only needs to live
    /// through this call; encoded bytes are self-contained.
    async fn send<'a, T: Facet<'a>>(self, item: &'a T) -> Result<(), Self::Error>;
}

/// Receiving half of a [`Conduit`].
///
/// Yields decoded values as [`SelfRef<T>`] (value + backing storage).
pub trait ConduitRx: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Receive and decode the next message.
    ///
    /// Flow:
    /// 1. `link_rx.recv()` → `Backing` (raw bytes + ownership)
    /// 2. `SelfRef::try_new(backing, |bytes| codec.decode(bytes))` → `SelfRef<T>`
    ///
    /// The caller specifies `T` — the codec decodes into it using the
    /// type's plan. For RPC, this is typically `Message`.
    ///
    /// Returns `Ok(None)` when the peer has closed.
    async fn recv<T: Facet<'static> + 'static>(
        &mut self,
    ) -> Result<Option<SelfRef<T>>, Self::Error>;
}

// ---------------------------------------------------------------------------
// Packet — internal to StableConduit
// ---------------------------------------------------------------------------

/// Packet sequence number (per direction).
#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
#[facet(transparent)]
pub struct PacketSeq(pub u32);

/// Cumulative ACK: all packets up to `max_delivered` (inclusive) have been
/// delivered to the upper layer.
#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PacketAck {
    pub max_delivered: PacketSeq,
}

/// Opaque session identifier for reliability-layer resume.
///
/// Assigned by the server on first connection. Sent by the client on
/// reconnect so the server can route the new raw link to the correct
/// StableConduit instance.
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResumeKey(pub Vec<u8>);

/// Client's opening handshake, sent as the first message on a new connection.
///
/// The server reads this to determine whether the connection is new or a
/// reconnect, and routes accordingly.
#[derive(Facet, Debug, Clone)]
pub struct ClientHello {
    /// `None` = new session (server assigns a key).
    /// `Some` = reconnect to existing session.
    pub resume_key: Option<ResumeKey>,

    /// Last contiguous seq delivered to this peer's upper layer.
    /// The other side replays everything after this.
    pub last_received: Option<PacketSeq>,
}

/// Server's handshake response.
///
/// Always carries a resume key (assigned for new sessions, confirmed for
/// reconnects).
#[derive(Facet, Debug, Clone)]
pub struct ServerHello {
    /// The session's resume key. Always present.
    pub resume_key: ResumeKey,

    /// Last contiguous seq delivered to this peer's upper layer.
    /// The other side replays everything after this.
    pub last_received: Option<PacketSeq>,
}

/// Sequenced data frame, serialized by StableConduit over a raw Link.
///
/// Handshake messages (ClientHello / ServerHello) are exchanged before
/// data flow begins — they're separate types, not part of this frame.
/// After the handshake, all traffic is `Frame<T>`.
#[derive(Facet, Debug, Clone)]
pub struct Frame<T> {
    pub seq: PacketSeq,
    pub ack: Option<PacketAck>,
    pub item: T,
}

// ---------------------------------------------------------------------------
// Attachment / LinkSource — for StableConduit reconnect
// ---------------------------------------------------------------------------

/// A raw link bundled with the peer's Hello (if already consumed).
///
/// - **Client** (`peer_hello = None`): Dialer connected, no Hello yet.
/// - **Server** (`peer_hello = Some`): acceptor already read client's Hello.
pub struct Attachment<L> {
    pub link: L,
    pub client_hello: Option<ClientHello>,
}

/// Source of replacement [`Link`]s for [`StableConduit`] reconnect.
///
/// - **Client (pull)**: Dialer that connects and returns a new link.
/// - **Server (push)**: channel receiver from the acceptor.
pub trait LinkSource: Send + 'static {
    type Link: Link;

    async fn next_link(&mut self) -> std::io::Result<Attachment<Self::Link>>;
}

// ---------------------------------------------------------------------------
// SessionAcceptor
// ---------------------------------------------------------------------------

/// Yields new sessions from inbound connections.
///
/// The acceptor listens for incoming connections and produces ready-to-use
/// [`Conduit`]s. For StableConduit-based transports, reconnects are handled
/// internally — only genuinely new sessions surface through `accept`.
///
/// User code calls `accept` in a loop and hands each Conduit to
/// `Session::new`.
pub trait SessionAcceptor {
    type Conduit: Conduit;

    async fn accept(&mut self) -> std::io::Result<Self::Conduit>;
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// Whether the session is acting as initiator or acceptor.
///
/// Determines who speaks first in the protocol handshake. Orthogonal to
/// reconnect — reconnect is handled by StableConduit, not Session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    Initiator,
    Acceptor,
}
