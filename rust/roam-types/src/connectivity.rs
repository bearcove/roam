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
//! - **Link provides write buffers** — `reserve()` yields a permit, then
//!   `permit.alloc(len)` returns a `WriteSlot` backed by the transport's own
//!   memory (bipbuffer for SHM, write buf for TCP). Caller encodes directly
//!   into the slot. One copy, not two.
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

/// A permit for allocating exactly one outbound payload.
///
/// Returned by [`LinkTx::reserve`]. The permit represents *message-level*
/// capacity (not bytes). Once you have a permit, turning it into a concrete
/// buffer for a specific payload size is synchronous.
pub trait LinkTxPermit {
    type Slot: WriteSlot;

    /// Allocate a writable buffer of exactly `len` bytes.
    ///
    /// This is synchronous once the permit has been acquired.
    fn alloc(self, len: usize) -> std::io::Result<Self::Slot>;
}

/// Sending half of a [`Link`].
///
/// Uses a two-phase write:
///
/// 1. [`reserve`](LinkTx::reserve) awaits until the transport can accept *one*
///    more payload and returns a [`LinkTxPermit`].
/// 2. [`LinkTxPermit::alloc`] allocates a [`WriteSlot`] backed by the
///    transport's own buffer (bipbuffer slot, kernel write buffer, etc.),
///    then the caller fills it and calls [`WriteSlot::commit`].
///
/// `reserve` is the backpressure point.
pub trait LinkTx: Send + 'static {
    type Permit: LinkTxPermit;

    /// Reserve capacity to send exactly one payload.
    ///
    /// Backpressure lives here — it awaits until the transport can accept a
    /// payload (or errors).
    ///
    /// Dropping the returned permit without allocating/committing MUST
    /// release the reservation.
    async fn reserve(&self) -> std::io::Result<Self::Permit>;

    /// Graceful close of the outbound direction.
    async fn close(self) -> std::io::Result<()>
    where
        Self: Sized;
}

/// A writable slot in the transport's output buffer.
///
/// Obtained from [`LinkTxPermit::alloc`]. The caller writes encoded bytes into
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
/// `send` is synchronous. All backpressure is applied by
/// [`ConduitTx::reserve`], which may await until both:
/// - the conduit is willing to accept another outstanding message, and
/// - the underlying link can accept one more payload.
pub trait ConduitTxPermit<T: 'static> {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Serialize `item` and send it through the link.
    ///
    /// Flow:
    /// 1. Encode item with facet-postcard → `Vec<u8>`
    /// 2. Allocate a write slot from a pre-reserved link permit
    /// 3. Copy into slot
    /// 4. Commit (+ buffer encoded bytes for replay in StableConduit)
    fn send(self, item: &T) -> Result<(), Self::Error>;
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
pub trait ConduitAcceptor<T: 'static> {
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
