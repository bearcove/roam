//! Byte-stream transport for roam.
//!
//! Implements [`Link<T, C>`](roam_types::Link) over any `AsyncRead + AsyncWrite`
//! pair (TCP, Unix sockets, stdio) using 4-byte little-endian length-prefix
//! framing.

/// A [`Link`](roam_types::Link) over a byte stream with length-prefix framing.
///
/// Wraps an `AsyncRead + AsyncWrite` pair. Each message is framed as
/// `[len: u32 LE][payload bytes]`.
pub struct StreamLink<R, W> {
    _reader: R,
    _writer: W,
}

/// Sending half of a [`StreamLink`].
pub struct StreamLinkTx<W> {
    _writer: W,
}

/// Receiving half of a [`StreamLink`].
pub struct StreamLinkRx<R> {
    _reader: R,
}
