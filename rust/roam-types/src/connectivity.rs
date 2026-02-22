//! Connectivity-layer API sketches.
//!
//! This module is intentionally "runtime-agnostic": it does not pick an async
//! runtime (Tokio, async-std, etc.). It also does not implement any protocol
//! state machine; it only defines the trait boundaries we want the Rust stack
//! to converge on.
//!
//! Layer names and trait names are intended to match:
//!
//! - **Link**: transports move opaque [`Payload`] units between peers.
//! - **Codec**: serialization between [`crate::Message`] and [`Payload`].
//! - **Wire**: a schema-blind wrapper over `Link` that sends/receives `Message`.
//! - **Session**: the protocol state machine (handshake, resume, routing, etc.).
//! - **Client**: generated service clients built on top of `Session`.

use crate::Payload;
use std::io;

/// A bidirectional established transport between two peers.
///
/// A transport (TCP, WebSocket, SHM, etc.) implements this trait to expose an
/// already-established pipe of [`Payload`] units.
///
/// A `Link` is owned by the protocol runtime task (the `Session` layer); user
/// code should generally not interact with `Link` directly.
pub trait Link {
    type Sender: LinkSender;
    type Receiver: LinkReceiver;

    fn split(self) -> (Self::Sender, Self::Receiver);
}

/// Sending side of a [`Link`].
///
/// Backpressure is expressed via `reserve()`: a successful reserve grants
/// capacity for exactly one `Payload` and yields a [`LinkSendPermit`].
pub trait LinkSender: Send + 'static {
    type Permit<'a>: LinkSendPermit
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

/// A permit for sending exactly one [`Payload`].
///
/// This MUST NOT block: backpressure happens at `reserve()`, not at `send()`.
pub trait LinkSendPermit {
    fn send(self, payload: Payload);
}

/// Receiving side of a [`Link`].
///
/// `recv` is single-consumer: it takes `&mut self`. Higher layers that need
/// fanout MUST implement it above the `Link` boundary.
pub trait LinkReceiver: Send + 'static {
    #[allow(async_fn_in_trait)]
    async fn recv(&mut self) -> io::Result<Option<Payload>>;
}

/// Message serialization and deserialization.
///
/// This is the only layer that understands the schema of `Message` at the
/// `Payload` boundary (postcard in Rust).
///
/// It MUST NOT implement request correlation, channel routing, flow control, or
/// reconnection logic. It is purely a `(Message ↔ Payload)` transform.
pub trait Codec: Send + Sync + 'static {
    type EncodeError: std::error::Error + Send + Sync + 'static;
    type DecodeError: std::error::Error + Send + Sync + 'static;

    fn encode(&self, message: &crate::Message) -> Result<Payload, Self::EncodeError>;
    fn decode(&self, payload: &Payload) -> Result<crate::Message, Self::DecodeError>;
}

/// Message-level IO over an established [`Link`].
///
/// This layer is protocol-message-shaped but schema-blind: it moves
/// `crate::Message` values without owning any call/session policy.
///
/// A typical implementation wraps a `Link` and a `Codec`.
pub trait Wire {
    type Sender: WireSender;
    type Receiver: WireReceiver;

    fn split(self) -> (Self::Sender, Self::Receiver);
}

pub trait WireSender: Send + 'static {
    type Permit<'a>: WireSendPermit
    where
        Self: 'a;

    #[allow(async_fn_in_trait)]
    async fn reserve(&self) -> io::Result<Self::Permit<'_>>;

    #[allow(async_fn_in_trait)]
    async fn close(self) -> io::Result<()>
    where
        Self: Sized;
}

pub trait WireSendPermit {
    fn send(self, message: crate::Message);
}

pub trait WireReceiver: Send + 'static {
    #[allow(async_fn_in_trait)]
    async fn recv(&mut self) -> io::Result<Option<crate::Message>>;
}

/// A source of new [`Link`] values (used for reconnect-capable initiators).
///
/// If a session is constructed from a single already-established `Link`, it
/// cannot reconnect unless it also has a `Dialer`.
pub trait Dialer: Send + Sync + 'static {
    type Wire: Wire;

    #[allow(async_fn_in_trait)]
    async fn dial(&self) -> io::Result<Self::Wire>;
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
pub struct SessionBuilder<W, D> {
    role: SessionRole,
    reconnect: Reconnect,
    wire: W,
    initial_wire: Option<W>,
    dialer: Option<D>,
}

impl<W, D> SessionBuilder<W, D> {
    pub fn new(role: SessionRole, wire: W) -> Self {
        Self {
            role,
            reconnect: Reconnect::Disabled,
            wire,
            initial_wire: None,
            dialer: None,
        }
    }

    pub fn reconnect(mut self, reconnect: Reconnect) -> Self {
        self.reconnect = reconnect;
        self
    }

    pub fn wire_instance(mut self, wire: W) -> Self {
        self.initial_wire = Some(wire);
        self
    }

    pub fn dialer(mut self, dialer: D) -> Self {
        self.dialer = Some(dialer);
        self
    }

    pub fn wire(&self) -> &W {
        &self.wire
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
pub struct SessionHandle<W, D> {
    _phantom: core::marker::PhantomData<(W, D)>,
}
