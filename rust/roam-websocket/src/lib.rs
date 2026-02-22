//! WebSocket transport for roam.
//!
//! Implements [`Link<T, C>`](roam_types::Link) over a WebSocket connection
//! using tungstenite. Each roam message maps 1:1 to a WebSocket binary frame.

/// A [`Link`](roam_types::Link) over a WebSocket connection.
pub struct WsLink<S> {
    _stream: S,
}

/// Sending half of a [`WsLink`].
pub struct WsLinkTx<S> {
    _sink: S,
}

/// Receiving half of a [`WsLink`].
pub struct WsLinkRx<S> {
    _stream: S,
}
