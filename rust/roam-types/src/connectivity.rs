//! Connectivity layer: Link (raw bytes) + Conduit (typed).
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
use std::mem::ManuallyDrop;

// ---------------------------------------------------------------------------
// SelfRef + Backing
// ---------------------------------------------------------------------------

/// A decoded value `T` that may borrow from its own backing storage.
///
/// Transports decode into storage they own (heap buffer, VarSlot, mmap).
/// `SelfRef` keeps that storage alive so `T` can safely borrow from it
/// (via Facet's `'static` lifetime + variance guarantee).
///
/// Uses `ManuallyDrop` + custom `Drop` to guarantee drop order: value is
/// dropped before backing, so borrowed references in `T` remain valid
/// through `T`'s drop.
///
/// `T` must be covariant in any lifetime parameters (checked at construction
/// via facet's variance tracking).
pub struct SelfRef<T: 'static> {
    /// The decoded value, potentially borrowing from `backing`.
    value: ManuallyDrop<T>,

    /// Backing storage keeping decoded bytes alive.
    backing: ManuallyDrop<Backing>,
}

/// Backing storage for a [`SelfRef`].
pub enum Backing {
    /// Heap-allocated buffer (TCP read, BipBuffer copy-out for small messages).
    Boxed(Box<[u8]>),
    // SHM VarSlot, pinned in shared memory:
    // VarSlot(Arc<VarSlot>),
    // Memory-mapped file region:
    // Mmap(Arc<MmapRegion>),
}

impl Backing {
    /// Access the backing bytes.
    fn as_bytes(&self) -> &[u8] {
        match self {
            Backing::Boxed(b) => b,
        }
    }
}

impl<T: 'static> Drop for SelfRef<T> {
    fn drop(&mut self) {
        // Drop value first (it may borrow from backing), then backing.
        unsafe {
            ManuallyDrop::drop(&mut self.value);
            ManuallyDrop::drop(&mut self.backing);
        }
    }
}

impl<T: 'static + Facet<'static>> SelfRef<T> {
    /// Construct a `SelfRef` from backing storage and a builder.
    ///
    /// The builder receives a `&'static [u8]` view of the backing bytes —
    /// sound because the backing is heap-allocated (stable address) and
    /// dropped after the value.
    ///
    /// Panics if `T` is not covariant (lifetime cannot safely shrink).
    pub fn try_new<E>(
        backing: Backing,
        builder: impl FnOnce(&'static [u8]) -> Result<T, E>,
    ) -> Result<Self, E> {
        let variance = T::SHAPE.computed_variance();
        assert!(
            variance.can_shrink(),
            "SelfRef<T> requires T to be covariant. Type {:?} has variance {:?}",
            T::SHAPE.type_identifier,
            variance
        );

        // Create a 'static slice from the backing bytes.
        // Sound because:
        // - Backing is heap-allocated (stable address)
        // - We drop value before backing (custom Drop impl)
        let bytes: &'static [u8] = unsafe {
            let b = backing.as_bytes();
            std::slice::from_raw_parts(b.as_ptr(), b.len())
        };

        let value = builder(bytes)?;

        Ok(Self {
            value: ManuallyDrop::new(value),
            backing: ManuallyDrop::new(backing),
        })
    }

    /// Infallible variant of [`try_new`](Self::try_new).
    pub fn new(backing: Backing, builder: impl FnOnce(&'static [u8]) -> T) -> Self {
        Self::try_new(backing, |bytes| {
            Ok::<_, std::convert::Infallible>(builder(bytes))
        })
        .unwrap_or_else(|e: std::convert::Infallible| match e {})
    }
}

impl<T: 'static> SelfRef<T> {
    /// Wrap an owned value that does NOT borrow from backing.
    ///
    /// No variance check — the value is fully owned. The backing is kept
    /// alive but the value doesn't reference it. Useful for in-memory
    /// transports (MemoryLink) where no deserialization occurs.
    pub fn owning(backing: Backing, value: T) -> Self {
        Self {
            value: ManuallyDrop::new(value),
            backing: ManuallyDrop::new(backing),
        }
    }

    /// Transform the contained value, keeping the same backing storage.
    ///
    /// Useful for projecting through wrapper types:
    /// `SelfRef<Frame<T>>` → `SelfRef<T>` by extracting the inner item.
    ///
    /// The closure receives the old value by move and returns the new value.
    /// Any references the new value holds into the backing storage (inherited
    /// from fields of `T`) remain valid — the backing is preserved.
    pub fn map<U: 'static>(mut self, f: impl FnOnce(T) -> U) -> SelfRef<U> {
        // SAFETY: we take both fields via ManuallyDrop::take, then forget
        // self to prevent its Drop impl from double-dropping them.
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        let backing = unsafe { ManuallyDrop::take(&mut self.backing) };
        core::mem::forget(self);

        SelfRef {
            value: ManuallyDrop::new(f(value)),
            backing: ManuallyDrop::new(backing),
        }
    }
}

impl<T: 'static> core::ops::Deref for SelfRef<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

// No `into_inner()` — T may borrow from backing. Use Deref instead.
// No `DerefMut` — mutating T could invalidate borrowed references.

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
/// Generic over `T`: the message type flowing through. The constructor
/// takes a `TypePlan<T>` (for type safety), type-erases it to
/// `TypePlanCore`, and uses the precomputed plan for fast deserialization.
///
/// Session is generic over this trait. The codec (postcard) is an
/// implementation detail — it doesn't appear in the trait signature.
///
/// Two implementations:
/// - `BareConduit`: Link + postcard. If the link dies, it's dead.
/// - `StableConduit`: Link + postcard + seq/ack/replay. Handles reconnect
///   transparently. Replay buffer stores encoded bytes (no `T: Clone`).
pub trait Conduit<T: 'static> {
    type Tx: ConduitTx<T>;
    type Rx: ConduitRx<T>;

    fn split(self) -> (Self::Tx, Self::Rx);
}

/// Sending half of a [`Conduit`].
///
/// Permit-based: `reserve()` is the backpressure point, `permit.send()`
/// serializes and writes.
pub trait ConduitTx<T: 'static>: Send + 'static {
    type Permit<'a>: ConduitTxPermit<T>
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
pub trait ConduitTxPermit<T: 'static> {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Serialize `item` and send it through the link.
    ///
    /// Flow:
    /// 1. Encode item with facet-postcard → `Vec<u8>`
    /// 2. `link_tx.alloc(len)` → write slot
    /// 3. Copy into slot
    /// 4. Commit (+ buffer encoded bytes for replay in StableConduit)
    async fn send(self, item: &T) -> Result<(), Self::Error>;
}

/// Receiving half of a [`Conduit`].
///
/// Yields decoded values as [`SelfRef<T>`] (value + backing storage).
/// Uses a precomputed `TypePlanCore` for fast plan-driven deserialization
/// via `Partial::from_raw` + `FormatDeserializer::deserialize_into`.
pub trait ConduitRx<T: 'static>: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Receive and decode the next message.
    ///
    /// Flow:
    /// 1. `link_rx.recv()` → `Backing` (raw bytes + ownership)
    /// 2. Allocate `MaybeUninit<T>`, create `Partial::from_raw` with cached plan
    /// 3. `PostcardParser` + `FormatDeserializer::deserialize_into`
    /// 4. Wrap in `SelfRef<T>`
    ///
    /// Returns `Ok(None)` when the peer has closed.
    async fn recv(&mut self) -> Result<Option<SelfRef<T>>, Self::Error>;
}

// ---------------------------------------------------------------------------
// Frame — internal to StableConduit
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
/// - **Client** (`client_hello = None`): Dialer connected, no Hello yet.
/// - **Server** (`client_hello = Some`): acceptor already read client's Hello.
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
pub trait SessionAcceptor<T: 'static> {
    type Conduit: Conduit<T>;

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
