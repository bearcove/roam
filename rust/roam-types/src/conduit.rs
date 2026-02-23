#![allow(async_fn_in_trait)]

use crate::SelfRef;

/// Bidirectional typed transport. Wraps a [`Link`](crate::Link) and owns serialization.
///
/// Generic over `T`: the message type flowing through. The constructor
/// takes a `TypePlan<T>` (for type safety), type-erases it to
/// `TypePlanCore`, and uses the precomputed plan for fast deserialization.
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
pub trait ConduitTxPermit<T: 'static> {
    type Error: std::error::Error + Send + Sync + 'static;

    fn send(self, item: &T) -> Result<(), Self::Error>;
}

/// Receiving half of a [`Conduit`].
///
/// Yields decoded values as [`SelfRef<T>`] (value + backing storage).
/// Uses a precomputed `TypePlanCore` for fast plan-driven deserialization.
pub trait ConduitRx<T: 'static>: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Receive and decode the next message.
    ///
    /// Returns `Ok(None)` when the peer has closed.
    async fn recv(&mut self) -> Result<Option<SelfRef<T>>, Self::Error>;
}

/// Yields new conduits from inbound connections.
pub trait ConduitAcceptor<T: 'static> {
    type Conduit: Conduit<T>;

    async fn accept(&mut self) -> std::io::Result<Self::Conduit>;
}

/// Whether the session is acting as initiator or acceptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    Initiator,
    Acceptor,
}
