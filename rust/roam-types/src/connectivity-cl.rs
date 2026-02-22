//! Connectivity-layer sketch (CL variant).
//!
//! Design:
//!
//! - **Link\<T, C\>**: bidirectional transport, generic over item type `T` and
//!   codec `C`. Transports (TCP, WS, SHM) implement this generically — they
//!   don't know what `T` is.
//! - **Asymmetric IO**: send `T`, receive [`SelfRef<T>`] (decoded value +
//!   refcounted backing storage for zero-copy).
//! - **Packet\<T\>**: wire frame struct (seq + ack + T). Implementation detail
//!   of `ReliableLink` — upper layers never see it.
//! - **ReliableLink**: wraps `Link<Packet<T>, C>`, strips packet framing,
//!   exposes `Link<T, C>` with transparent reconnect + replay.
//! - **Codec**: plan-driven, scatter-gather encode, unsafe `decode_into`. Not
//!   generic over `T`.
//! - **Session**: knows `Message` concretely, generic over the `Link`.

use facet::Facet;
use std::io;

// ---------------------------------------------------------------------------
// Codec
// ---------------------------------------------------------------------------

/// A chunk of encoded output from [`Codec::encode_scatter`].
pub enum EncodedPart<'a> {
    /// Small skeleton bytes (struct layout, varint prefixes, seq/ack metadata).
    /// Owned by the codec's traversal.
    Skeleton(smallvec::SmallVec<[u8; 64]>),

    /// Reference to existing large data in the source value (e.g. the contents
    /// of a `Vec<u8>` field). Borrowed from the value being encoded — valid for
    /// lifetime `'a`.
    BorrowedBlob(&'a [u8]),
}

/// Plan-driven serialization and deserialization.
///
/// A single codec instance (e.g. `PostcardCodec`) handles any `T: Facet` via
/// precomputed [`facet::TypePlanCore`].
///
/// Not generic over `T`: the plan carries the type information.
pub trait Codec: Send + Sync + 'static {
    type EncodeError: std::error::Error + Send + Sync + 'static;
    type DecodeError: std::error::Error + Send + Sync + 'static;

    /// Scatter-gather encode: serialize a value into a list of parts.
    ///
    /// The codec traverses the value (via [`facet_reflect::Peek`]), serializes
    /// small fields into [`EncodedPart::Skeleton`] chunks, and records
    /// references to large byte slices as [`EncodedPart::BorrowedBlob`].
    ///
    /// Returns the total encoded size (sum of all parts). The caller uses
    /// this to reserve space in the destination (BipBuffer, VarSlot, etc.),
    /// then writes the parts sequentially. One traversal, large blobs copied
    /// once directly from source to destination.
    ///
    /// `value` is a [`facet_reflect::Peek`] — a safe handle carrying both the
    /// pointer and the type's shape. No raw pointer / plan mismatch possible.
    fn encode_scatter<'a>(
        &self,
        value: facet_reflect::Peek<'a, 'a>,
        parts: &mut Vec<EncodedPart<'a>>,
    ) -> Result<usize, Self::EncodeError>;

    /// Decode bytes into uninitialized memory using a precomputed plan.
    ///
    /// # Safety
    ///
    /// - `out` must be valid, aligned, and properly-sized for `plan`'s root shape.
    /// - On error, the codec MUST clean up any partially-initialized fields
    ///   (via facet's [`facet_reflect::Partial`] or equivalent).
    unsafe fn decode_into(
        &self,
        plan: &facet::TypePlanCore,
        bytes: &[u8],
        out: facet_core::PtrUninit,
    ) -> Result<(), Self::DecodeError>;
}

// ---------------------------------------------------------------------------
// SelfRef
// ---------------------------------------------------------------------------

/// A decoded value `T` that may borrow from its own backing storage.
///
/// Transports decode into storage they own (heap buffer, VarSlot, mmap).
/// `SelfRef` keeps that storage alive so `T` can safely borrow from it
/// (via Facet's `'static` lifetime + variance guarantee).
///
/// Drop order: `value` is dropped before `_backing`, so borrowed references
/// in `T` remain valid through `T`'s drop.
pub struct SelfRef<T> {
    /// The decoded value, potentially borrowing from `_backing`.
    /// Dropped first.
    value: T,

    /// Backing storage keeping decoded bytes alive.
    /// Dropped second.
    _backing: Backing,
}

/// Backing storage for a [`SelfRef`].
enum Backing {
    /// Heap-allocated buffer (TCP read, BipBuffer copy-out for small messages).
    Boxed(Box<[u8]>),
    /// SHM VarSlot, pinned in shared memory.
    // VarSlot(Arc<VarSlot>),
    /// Memory-mapped file region.
    // Mmap(Arc<MmapRegion>),
}

impl<T> core::ops::Deref for SelfRef<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> core::ops::DerefMut for SelfRef<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

// No `into_inner()` — T may borrow from backing. Use Deref instead.

// ---------------------------------------------------------------------------
// Link
// ---------------------------------------------------------------------------

/// Bidirectional transport of `T` values using codec `C`.
///
/// Transports implement this generically — a TCP link doesn't know whether
/// `T` is `Message`, `Packet<Message>`, or anything else. It just encodes `T`
/// via `C` into its write buffer and decodes from its read buffer.
///
/// **Asymmetric**: [`LinkTx`] sends `T`, [`LinkRx`] yields [`SelfRef<T>`].
///
/// Composable: `ReliableLink` wraps `Link<Packet<T>, C>` and itself implements
/// `Link<T, C>`, hiding the packet layer from everything above.
pub trait Link<T, C: Codec> {
    type Tx: LinkTx<T, C>;
    type Rx: LinkRx<T, C>;

    fn split(self) -> (Self::Tx, Self::Rx);
}

/// Sending half of a [`Link`].
///
/// Permit-based backpressure: `reserve()` awaits capacity for one item.
pub trait LinkTx<T, C: Codec>: Send + 'static {
    type Permit<'a>: LinkTxPermit<T, C>
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

/// Permit for sending exactly one item. MUST NOT block (beyond encoding).
///
/// The permit knows the codec `C` and can return typed encode errors.
pub trait LinkTxPermit<T, C: Codec> {
    fn send(self, item: T) -> Result<(), C::EncodeError>;
}

/// Receiving half of a [`Link`].
///
/// Yields [`SelfRef<T>`]: decoded value + backing storage.
/// Single-consumer (`&mut self`).
pub trait LinkRx<T, C: Codec>: Send + 'static {
    /// Receive the next item.
    ///
    /// Decode errors are mapped to `io::Error` — a decode failure means the
    /// link is broken (per spec: deserialization failure = link failure).
    #[allow(async_fn_in_trait)]
    async fn recv(&mut self) -> io::Result<Option<SelfRef<T>>>;
}

// ---------------------------------------------------------------------------
// Packet (struct, not trait — internal to ReliableLink)
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

/// Wire frame: seq/ack metadata wrapping a payload of type `T`.
///
/// Serialized atomically by the codec in one pass — no intermediate buffer.
/// Only exists inside `ReliableLink`; upper layers see `T` directly.
#[derive(Facet, Debug, Clone)]
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
//         rx: L::Rx,     // receives SelfRef<Message>
//         role: SessionRole,
//         ...
//     }
//
// Whether L is raw SHM, TCP + ReliableLink, or anything else is invisible.
// Session sends Message, receives SelfRef<Message>, runs the protocol
// state machine (handshake, connections, request correlation, channels).
