//! Connectivity-layer API sketches.
//!
//! This module is intentionally "runtime-agnostic": it does not pick an async
//! runtime (Tokio, async-std, etc.). It also does not implement any protocol
//! state machine; it only defines the trait boundaries we want the Rust stack
//! to converge on.
//!
//! Layer names and trait names are intended to match:
//!
//! - **Link**: transports move *typed values* between peers, applying
//!   backpressure via permits. Transports are generic over the value type and
//!   do not need to "know" what that type is beyond generic bounds.
//! - **ReliableLink**: provides reliable, ordered delivery and resumption for a
//!   `Link<T>` by wrapping items in [`Packet<T>`] internally (sequence numbers
//!   and acknowledgements). The `Packet<T>` envelope is an implementation
//!   detail below the reliability boundary; upper layers see only `T`.
//! - **Codec**: serialization and deserialization used by `Link` to write/read
//!   values directly into/from transport storage (socket buffers, SHM regions,
//!   etc.). The concrete `Codec` (postcard) is chosen by higher layers; `Link`
//!   stays generic over it.
//! - **Session**: the protocol state machine (handshake, resume, routing, etc.).
//! - **Client**: generated service clients built on top of `Session`.

use facet::Facet;
use facet_reflect::TypePlanCore;
use std::io;

/// A bidirectional established transport between two peers.
///
/// A transport (TCP, WebSocket, SHM, etc.) implements this trait to expose an
/// already-established pipe of items.
///
/// A `Link` is owned by the protocol runtime task (the `Session` layer); user
/// code should generally not interact with `Link` directly.
pub trait Link<TxItem, RxItem, C>
where
    TxItem: for<'a> Facet<'a>,
    RxItem: for<'a> Facet<'a>,
    C: Codec,
{
    type Tx: LinkTx<TxItem>;
    type Rx: LinkRx<RxItem>;

    fn split(self) -> (Self::Tx, Self::Rx);
}

/// Sending side of a [`Link`].
///
/// Backpressure is expressed via `reserve()`: a successful reserve grants
/// capacity for exactly one item and yields a [`LinkTxPermit`].
pub trait LinkTx<T>: Send + 'static {
    type Permit<'a>: LinkTxPermit<T>
    where
        Self: 'a;

    /// Wait for outbound capacity for exactly one payload and reserve it for
    /// the caller.
    ///
    /// Cancellation of `reserve()` MUST NOT leak capacity; if reservation
    /// succeeds, dropping the returned permit MUST return that capacity.
    #[allow(async_fn_in_trait)]
    async fn reserve(&self) -> io::Result<Self::Permit<'_>>;

    /// Request a graceful close of the outbound direction.
    ///
    /// This consumes `self` so it cannot be called twice.
    #[allow(async_fn_in_trait)]
    async fn close(self) -> io::Result<()>
    where
        Self: Sized;
}

/// A permit for sending exactly one item.
///
/// This MUST NOT block: backpressure happens at `reserve()`, not at `send()`.
pub trait LinkTxPermit<T> {
    fn send(self, item: T);
}

/// Receiving side of a [`Link`].
///
/// `recv` is single-consumer: it takes `&mut self`. Higher layers that need
/// fanout MUST implement it above the `Link` boundary.
pub trait LinkRx<T>: Send + 'static {
    #[allow(async_fn_in_trait)]
    async fn recv(&mut self) -> io::Result<Option<T>>;
}

/// Packet sequence number (per direction).
///
/// This sequence space is used to implement:
/// - replay after reconnect
/// - cumulative ACK (highest delivered sequence)
/// - bounded buffering with backpressure (no silent drop)
///
/// Note: this is an API sketch; the exact representation is not fixed here.
#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
#[facet(transparent)]
pub struct PacketSeq(pub u32);

/// Cumulative ACK: all packets up to (and including) `max_delivered` have been
/// delivered to the upper layer.
///
/// This is intentionally cumulative (not SACK/ranges) to keep the packet layer
/// simple. Transports that can reorder may still buffer internally, but the
/// packet layer only reports contiguous delivery progress.
#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PacketAck {
    pub max_delivered: PacketSeq,
}

/// A packet envelope around one item.
///
/// This is what the packet layer sends on the wire. Making this generic over
/// `T` lets the packet layer avoid "double serialization": a transport can
/// encode `Packet<T>` directly in one pass.
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
pub struct Packet<T> {
    pub seq: PacketSeq,
    pub ack: Option<PacketAck>,
    pub item: T,
}

/// Sketch: a reliability wrapper that strips the `Packet<T>` envelope.
///
/// The reliability layer is generic over `T`: it understands `Packet` (seq/ack,
/// replay, dedup) but remains blind to `T` itself.
///
/// The wrapped link typically transports `Packet<T>` values, while the exposed
/// link transports `T` values.
#[derive(Debug)]
pub struct ReliableLink<L, T, C> {
    _inner: L,
    _phantom: core::marker::PhantomData<(T, C)>,
}

/// Message serialization and deserialization.
///
/// A codec is selected by higher layers (typically postcard for any `T: Facet`)
/// and used by [`Link`] implementations to encode and decode values.
///
/// This is intentionally an API sketch. The concrete design is expected to:
/// - support plan-driven encode/decode via [`TypePlanCore`] (prepared statement)
/// - support encode directly into transport-owned buffers (SHM bipbuffer,
///   varslots, socket buffers) without intermediate staging buffers when possible
pub trait Codec: Send + Sync + 'static {
    type EncodeError: std::error::Error + Send + Sync + 'static;
    type DecodeError: std::error::Error + Send + Sync + 'static;

    fn encode_by_plan(
        &self,
        plan: &TypePlanCore,
        value: facet::Peek<'_, '_>,
        out: &mut dyn io::Write,
    ) -> Result<(), Self::EncodeError>;

    /// Decode into an existing allocation using a precomputed plan.
    ///
    /// # Safety
    /// `out_ptr` must point to valid uninitialized storage for `plan.root()`.
    /// On error, the codec MUST NOT leak partially-initialized allocations.
    unsafe fn decode_into_by_plan(
        &self,
        plan: &TypePlanCore,
        payload: &[u8],
        out_ptr: *mut u8,
    ) -> Result<(), Self::DecodeError>;

    fn decode_by_plan<T: Facet<'static>>(
        &self,
        plan: &TypePlanCore,
        payload: &[u8],
    ) -> Result<T, Self::DecodeError>;
}

/// A source of new [`Link`] values (used for reconnect-capable initiators).
///
/// If a session is constructed from a single already-established `Link`, it
/// cannot reconnect unless it also has a `Dialer`.
pub trait Dialer<TxItem, RxItem, C>: Send + Sync + 'static
where
    TxItem: for<'a> Facet<'a>,
    RxItem: for<'a> Facet<'a>,
    C: Codec,
{
    type Link: Link<TxItem, RxItem, C>;

    #[allow(async_fn_in_trait)]
    async fn dial(&self) -> io::Result<Self::Link>;
}

/// Whether the session is acting as initiator (dialing) or acceptor (accepting).
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

/// A sketch of a `Session` builder.
///
/// This is a *shape*, not a full implementation: the actual state machine lives
/// in `roam-runtime`.
#[derive(Debug)]
pub struct SessionBuilder<L, D> {
    role: SessionRole,
    reconnect: Reconnect,
    initial_link: Option<L>,
    dialer: Option<D>,
}

impl<L, D> SessionBuilder<L, D> {
    pub fn new(role: SessionRole) -> Self {
        Self {
            role,
            reconnect: Reconnect::Disabled,
            initial_link: None,
            dialer: None,
        }
    }

    pub fn reconnect(mut self, reconnect: Reconnect) -> Self {
        self.reconnect = reconnect;
        self
    }

    pub fn link(mut self, link: L) -> Self {
        self.initial_link = Some(link);
        self
    }

    pub fn dialer(mut self, dialer: D) -> Self {
        self.dialer = Some(dialer);
        self
    }

    pub fn role(&self) -> SessionRole {
        self.role
    }

    pub fn reconnect_policy(&self) -> Reconnect {
        self.reconnect
    }
}

/// Placeholder "built" session type, for API-shape discussion.
///
/// The real session handle will live in `roam-runtime` and expose call/dispatch
/// APIs. This exists so we can discuss type parameters and builder ergonomics.
#[derive(Debug)]
pub struct SessionHandle<L, D> {
    _phantom: core::marker::PhantomData<(L, D)>,
}
