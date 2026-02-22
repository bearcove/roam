//! Connectivity-layer sketch (CL variant).
//!
//! Design:
//!
//! - **Link\<T, C\>**: bidirectional transport, generic over item type `T` and
//!   codec `C`. Transports (TCP, WS, SHM) implement this generically — they
//!   don't know what `T` is.
//! - **Asymmetric IO**: send `T`, receive [`OwnedMessage<T>`] (decoded value +
//!   refcounted backing storage for zero-copy).
//! - **Packet\<T\>**: wire frame struct (seq + ack + T). Implementation detail
//!   of `ReliableLink` — upper layers never see it.
//! - **ReliableLink**: wraps `Link<Packet<T>, C>`, strips packet framing,
//!   exposes `Link<T, C>` with transparent reconnect + replay.
//! - **Codec**: plan-driven, sink-based encode, unsafe `decode_into`. Not
//!   generic over `T`.
//! - **Session**: knows `Message` concretely, generic over the `Link`.

use std::io;

// ---------------------------------------------------------------------------
// Codec
// ---------------------------------------------------------------------------

/// Plan-driven serialization and deserialization.
///
/// A single codec instance (e.g. `PostcardCodec`) handles any `T: Facet` via
/// precomputed [`facet::TypePlanCore`]. It writes directly into a
/// caller-provided sink — the caller controls where bytes land (socket buffer,
/// BipBuffer region, VarSlot, `Vec`, etc.).
///
/// Not generic over `T`: the plan carries the type information.
pub trait Codec: Send + Sync + 'static {
    type EncodeError: std::error::Error + Send + Sync + 'static;
    type DecodeError: std::error::Error + Send + Sync + 'static;

    /// Encode a value into a sink.
    ///
    /// Returns the number of bytes written.
    fn encode_into(
        &self,
        plan: &facet::TypePlanCore,
        value: facet_core::PtrConst,
        sink: &mut dyn io::Write,
    ) -> Result<usize, Self::EncodeError>;

    /// Decode bytes into uninitialized memory.
    ///
    /// # Safety
    ///
    /// - `out` must be valid, aligned, and properly-sized for `plan`'s root shape.
    /// - On error, the codec MUST NOT leave partially-initialized allocations
    ///   (must clean up on failure).
    unsafe fn decode_into(
        &self,
        plan: &facet::TypePlanCore,
        bytes: &[u8],
        out: facet_core::PtrUninit,
    ) -> Result<(), Self::DecodeError>;
}

// ---------------------------------------------------------------------------
// OwnedMessage
// ---------------------------------------------------------------------------

/// A decoded value `T` paired with its backing storage.
///
/// Transports decode into storage they own (heap buffer, VarSlot, mmap).
/// `OwnedMessage` keeps that storage alive so `T` can safely borrow from it
/// (via Facet's `'static` lifetime + variance guarantee).
///
/// `Deref`s to `T` for transparent access. `into_inner()` when `T` is fully
/// owned (no borrows into backing).
pub struct OwnedMessage<T> {
    _backing: Backing,
    value: T,
}

enum Backing {
    /// Heap buffer (TCP read, BipBuffer copy-out for small messages).
    Boxed(Box<[u8]>),
    // Future:
    // VarSlot(Arc<VarSlot>),   — SHM pinned slot
    // Mmap(Arc<MmapRegion>),   — memory-mapped file
}

impl<T> core::ops::Deref for OwnedMessage<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> OwnedMessage<T> {
    /// Take the inner value, dropping the backing.
    ///
    /// Safe when `T` does not borrow from backing (all owned fields).
    pub fn into_inner(self) -> T {
        self.value
    }
}

// ---------------------------------------------------------------------------
// Link
// ---------------------------------------------------------------------------

/// Bidirectional transport of `T` values using codec `C`.
///
/// Transports implement this generically — a TCP link doesn't know whether
/// `T` is `Message`, `Packet<Message>`, or anything else. It just encodes `T`
/// via `C` into its write buffer and decodes from its read buffer.
///
/// **Asymmetric**: [`LinkTx`] sends `T`, [`LinkRx`] yields [`OwnedMessage<T>`].
///
/// Composable: `ReliableLink` wraps `Link<Packet<T>, C>` and itself implements
/// `Link<T, C>`, hiding the packet layer from everything above.
pub trait Link<T, C: Codec> {
    type Tx: LinkTx<T>;
    type Rx: LinkRx<T>;

    fn split(self) -> (Self::Tx, Self::Rx);
}

/// Sending half of a [`Link`].
///
/// Permit-based backpressure: `reserve()` awaits capacity for one item.
pub trait LinkTx<T>: Send + 'static {
    type Permit<'a>: LinkTxPermit<T>
    where
        Self: 'a;

    /// Reserve capacity for exactly one item.
    ///
    /// Cancellation MUST NOT leak capacity. Dropping a permit without calling
    /// `send()` MUST return that capacity.
    #[allow(async_fn_in_trait)]
    async fn reserve(&self) -> io::Result<Self::Permit<'_>>;

    /// Graceful close of the outbound direction. Consumes self.
    #[allow(async_fn_in_trait)]
    async fn close(self) -> io::Result<()>
    where
        Self: Sized;
}

/// Permit for sending exactly one item. MUST NOT block.
pub trait LinkTxPermit<T> {
    fn send(self, item: T);
}

/// Receiving half of a [`Link`].
///
/// Yields [`OwnedMessage<T>`]: decoded value + backing storage.
/// Single-consumer (`&mut self`).
pub trait LinkRx<T>: Send + 'static {
    #[allow(async_fn_in_trait)]
    async fn recv(&mut self) -> io::Result<Option<OwnedMessage<T>>>;
}

// ---------------------------------------------------------------------------
// Packet (struct, not trait — internal to ReliableLink)
// ---------------------------------------------------------------------------

/// Packet sequence number (per direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PacketSeq(pub u32);

/// Cumulative ACK: all packets up to `max_delivered` (inclusive) have been
/// delivered to the upper layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PacketAck {
    pub max_delivered: PacketSeq,
}

/// Wire frame: seq/ack metadata wrapping a payload of type `T`.
///
/// Serialized atomically by the codec in one pass — no intermediate buffer.
/// Only exists inside `ReliableLink`; upper layers see `T` directly.
#[derive(Debug, Clone)]
pub struct Packet<T> {
    pub seq: PacketSeq,
    pub ack: Option<PacketAck>,
    pub item: T,
}

// ---------------------------------------------------------------------------
// ReliableLink (concept, not implemented here)
// ---------------------------------------------------------------------------
//
// struct ReliableLink<T, C: Codec> { ... }
//
// impl<T, C: Codec> Link<T, C> for ReliableLink<T, C> { ... }
//
// Wraps a Link<Packet<T>, C>. Generic over T — doesn't know what T is.
// Understands Packet: assigns seq, processes acks, buffers for replay,
// deduplicates inbound. On reconnect (via Dialer), establishes a new
// underlying Link<Packet<T>, C> and replays unacked packets.
//
// Upper layers see Link<T, C>. Packet is invisible.

// ---------------------------------------------------------------------------
// Dialer
// ---------------------------------------------------------------------------

/// Source of new [`Link`] values for reconnect.
///
/// Used by `ReliableLink` to establish replacement links after failure.
pub trait Dialer<T, C: Codec>: Send + Sync + 'static {
    type Link: Link<T, C>;

    #[allow(async_fn_in_trait)]
    async fn dial(&self) -> io::Result<Self::Link>;
}

// ---------------------------------------------------------------------------
// Session types
// ---------------------------------------------------------------------------

/// Whether the session is acting as initiator or acceptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    Initiator,
    Acceptor,
}

/// Reconnect policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconnect {
    Disabled,
    Enabled,
}

// ---------------------------------------------------------------------------
// Session (shape)
// ---------------------------------------------------------------------------
//
// Session knows Message concretely but is generic over the Link:
//
//     struct Session<L, C>
//     where
//         C: Codec,
//         L: Link<crate::Message, C>,
//     {
//         tx: L::Tx,     // sends Message
//         rx: L::Rx,     // receives OwnedMessage<Message>
//         role: SessionRole,
//         ...
//     }
//
// Whether L is raw SHM, TCP + ReliableLink, or anything else is invisible.
// Session sends Message, receives OwnedMessage<Message>, runs the protocol
// state machine (handshake, connections, request correlation, channels).
