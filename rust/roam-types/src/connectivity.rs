//! Connectivity-layer sketch (CL variant).
//!
//! Design:
//!
//! - **Link\<T, C\>**: bidirectional transport, generic over item type `T` and
//!   codec `C`. Transports (TCP, WS, SHM) implement this generically — they
//!   don't know what `T` is.
//! - **Asymmetric IO**: send `T`, receive [`SelfRef<T>`] (decoded value +
//!   refcounted backing storage for zero-copy).
//! - **Packet\<T\>**: wire frame enum — either `Hello` (reliability handshake)
//!   or `Data` (seq + ack + T). Implementation detail of `ReliableLink` —
//!   upper layers never see it.
//! - **ReliableLink**: wraps `Link<Packet<T>, C>`, strips packet framing,
//!   exposes `Link<T, C>` with transparent reconnect + replay. Generic over
//!   [`LinkSource`] for how replacement links arrive (pull via Dialer on
//!   client, push via channel on server).
//! - **ReliableAcceptor**: server-side router. Accepts raw connections, reads
//!   the first `Packet::Hello` to extract a [`ResumeKey`], routes reconnects
//!   to existing `ReliableLink` instances, yields new sessions upward.
//! - **Codec**: plan-driven, scatter-gather encode, unsafe `decode_into`. Not
//!   generic over `T`. TypeErasedValue plan delegation is expanded by the codec
//!   into Skeleton + BorrowedBlob parts during traversal.
//! - **Session**: knows `Message` concretely, generic over the `Link`.

#![allow(unsafe_code)]

use facet::Facet;
use std::mem::ManuallyDrop;

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
    ///
    /// For TypeErasedValue fields (plan-delegation), the codec expands them
    /// into Skeleton + BorrowedBlob parts during traversal — no special part
    /// kind needed.
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
enum Backing {
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

impl<T: 'static> core::ops::Deref for SelfRef<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

// No `into_inner()` — T may borrow from backing. Use Deref instead.
// No `DerefMut` — mutating T could invalidate borrowed references.

// ---------------------------------------------------------------------------
// Link
// ---------------------------------------------------------------------------

/// Bidirectional transport of `T` values using codec `C`.
///
/// Transports implement this generically — a TCP link doesn't know whether
/// `T` is `Message`, `Packet<Message>`, or anything else. It just encodes `T`
/// via `C` into its write buffer and decodes from its read buffer.
///
/// **Asymmetric by definition**: [`LinkTx`] sends `T`, [`LinkRx`] yields
/// [`SelfRef<T>`]. So `Link<Message, C>` means Tx sends `Message` and Rx
/// yields `SelfRef<Message>`.
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
    async fn reserve(&self) -> std::io::Result<Self::Permit<'_>>;

    /// Graceful close of the outbound direction. Consumes self.
    #[allow(async_fn_in_trait)]
    async fn close(self) -> std::io::Result<()>
    where
        Self: Sized;
}

/// Permit for sending exactly one item. MUST NOT block (beyond encoding).
///
/// The permit knows the codec `C` and can return typed encode errors.
/// Note: "send never blocks" still holds — encoding can *fail*, but it
/// does not await or apply backpressure. Backpressure is at `reserve()`.
pub trait LinkTxPermit<T, C: Codec> {
    fn send(self, item: T) -> Result<(), C::EncodeError>;
}

/// Receiving half of a [`Link`].
///
/// Yields [`SelfRef<T>`]: decoded value + backing storage.
/// Single-consumer (`&mut self`).
pub trait LinkRx<T, C: Codec>: Send + 'static {
    /// Transport-specific error type (IO errors, decode errors, framing
    /// errors, peer death, etc.).
    type Error: std::error::Error + Send + Sync + 'static;

    /// Receive the next item.
    #[allow(async_fn_in_trait)]
    async fn recv(&mut self) -> Result<Option<SelfRef<T>>, Self::Error>;
}

// ---------------------------------------------------------------------------
// Packet (enum — internal to ReliableLink)
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
/// [`ReliableLink`] instance.
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResumeKey(pub Vec<u8>);

/// Reliability handshake exchanged as the first packet on a new connection.
///
/// Sent as `Packet::Hello` with seq=0. The server reads this to determine
/// whether the connection is new or a reconnect, and routes accordingly.
#[derive(Facet, Debug, Clone)]
pub struct ReliableHello {
    /// `None` = new session (server assigns a key).
    /// `Some` = reconnect to existing session.
    pub resume_key: Option<ResumeKey>,

    /// Last contiguous seq delivered to this peer's upper layer.
    /// The other side replays everything after this.
    pub last_received: Option<PacketSeq>,
}

/// Wire frame: the unit carried over a raw `Link` inside the reliability layer.
///
/// Serialized atomically by the codec in one pass — no intermediate buffer.
/// Only exists inside `ReliableLink`; upper layers see `T` directly.
///
/// The first packet on a new connection MUST be `Hello` (seq=0).
/// All subsequent packets are `Data`.
#[derive(Facet, Debug, Clone)]
pub enum Packet<T> {
    /// Reliability handshake (first packet, seq=0).
    Hello(ReliableHello),

    /// Sequenced data carrying an upper-layer item.
    Data {
        seq: PacketSeq,
        ack: Option<PacketAck>,
        item: T,
    },
}

// ---------------------------------------------------------------------------
// LinkSource
// ---------------------------------------------------------------------------

/// Source of replacement [`Link`] values for [`ReliableLink`].
///
/// Client and server differ only in how a replacement link arrives:
///
/// - **Client (pull)**: a `Dialer` that connects and returns a new link.
///   May error; caller decides retry policy.
/// - **Server (push)**: a channel receiver that awaits the next inbound
///   connection routed by [`ReliableAcceptor`]. Times out if no reconnect
///   arrives within a configured window.
///
/// `ReliableLink` is generic over this trait — same seq/ack/replay logic
/// regardless of which side initiated the connection.
pub trait LinkSource<T, C: Codec>: Send + 'static {
    type Link: Link<Packet<T>, C>;

    #[allow(async_fn_in_trait)]
    async fn next_link(&mut self) -> std::io::Result<Self::Link>;
}

// ---------------------------------------------------------------------------
// ReliableLink
// ---------------------------------------------------------------------------
//
// struct ReliableLink<T, C, S>
// where
//     C: Codec,
//     S: LinkSource<T, C>,
// {
//     source: S,
//     current_tx: <S::Link as Link<Packet<T>, C>>::Tx,
//     current_rx: <S::Link as Link<Packet<T>, C>>::Rx,
//     tx_seq: PacketSeq,
//     rx_seq: PacketSeq,
//     replay_buffer: VecDeque<...>,
//     ...
// }
//
// impl<T, C, S> Link<T, C> for ReliableLink<T, C, S> { ... }
//
// Generic over T — doesn't know what T is.
// Understands Packet: assigns seq, processes acks, buffers for replay,
// deduplicates inbound. On link failure, calls source.next_link() to
// obtain a replacement, exchanges Hello, replays unacked packets.
//
// Upper layers see Link<T, C>. Packet is invisible.

// ---------------------------------------------------------------------------
// ReliableAcceptor (server-side router)
// ---------------------------------------------------------------------------
//
// struct ReliableAcceptor<T, C, L>
// where
//     C: Codec,
//     L: Link<Packet<T>, C>,
// {
//     sessions: HashMap<ResumeKey, mpsc::Sender<L>>,
//     ...
// }
//
// The server's accept loop hands each new raw connection to
// ReliableAcceptor::ingest(raw_link):
//
// 1. Read the first Packet::Hello from the raw link.
// 2. Extract the ResumeKey.
// 3. If resume_key matches an existing session:
//    push the raw link into that session's mpsc::Sender<L>.
//    The corresponding ReliableLink is sitting on its LinkSource
//    (channel receiver) waiting for this.
// 4. If resume_key is None (new session):
//    assign a ResumeKey, create a new ReliableLink with a channel-based
//    LinkSource, yield the new session upward.
//
// User code:
//
//     loop {
//         let session = acceptor.accept().await;
//         tokio::spawn(handle(session));
//         // reconnects are routed internally, never surface here
//     }

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// Whether the session is acting as initiator or acceptor.
///
/// Determines who speaks first in the protocol handshake. Orthogonal to
/// reconnect — reconnect is handled by `ReliableLink`, not `Session`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    Initiator,
    Acceptor,
}

// Session knows Message concretely but is generic over the Link.
//
// Link<T, C> is asymmetric by definition: it sends T and receives
// SelfRef<T>. So L: Link<Message, C> means:
// - L::Tx sends Message
// - L::Rx yields SelfRef<Message>
//
//     struct Session<L, C>
//     where
//         C: Codec,
//         L: Link<crate::Message, C>,
//     {
//         tx: L::Tx,
//         rx: L::Rx,
//         role: SessionRole,
//         ...
//     }
//
// Session does not know or care about reliability. Whether L is a raw SHM
// link, a ReliableLink over TCP, or anything else is invisible. Session
// sends Message, receives SelfRef<Message>, runs the protocol state
// machine (handshake, connections, request correlation, channels).
//
// Construction:
//
// - SHM (no reliability needed):
//     Session::new(shm_link, role)
//
// - TCP client:
//     let reliable = ReliableLink::new(dialer, ...);
//     Session::new(reliable, Initiator)
//
// - TCP server (via ReliableAcceptor):
//     let session = acceptor.accept().await;
//     // internally: ReliableAcceptor created a ReliableLink with a
//     // channel-based LinkSource, wrapped it in Session(Acceptor)
