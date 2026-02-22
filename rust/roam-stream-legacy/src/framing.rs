//! Length-prefixed framing for async streams.
//!
//! r[impl transport.bytestream.length-prefix] - Messages are prefixed by a
//! 4-byte little-endian frame length.
//! r[impl transport.message.binary] - All messages are binary (not text).
//! r[impl transport.message.one-to-one] - Each frame contains exactly one roam message.
//! r[impl transport.message.multiplexing] - channel_id field provides multiplexing.
//!
//! This module is generic over the transport type - it works with any type that
//! implements `AsyncRead + AsyncWrite + Unpin`, including:
//! - `TcpStream` (TCP sockets)
//! - `UnixStream` (Unix domain sockets)
//! - Any other async byte stream

use std::io;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use roam_core::MessageTransport;
use roam_types::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Enable wire-level message logging for debugging.
/// Set ROAM_WIRE_SPY=1 to enable.
static WIRE_SPY_ENABLED: AtomicBool = AtomicBool::new(false);
static WIRE_SPY_BYTES_ONLY: AtomicBool = AtomicBool::new(false);

static WIRE_SPY_INIT: OnceLock<()> = OnceLock::new();

fn wire_spy_enabled() -> bool {
    WIRE_SPY_INIT.get_or_init(|| {
        if std::env::var("ROAM_WIRE_SPY").is_ok() {
            WIRE_SPY_ENABLED.store(true, Ordering::Relaxed);
        }
        if std::env::var("ROAM_WIRE_SPY_BYTES_ONLY").is_ok() {
            WIRE_SPY_BYTES_ONLY.store(true, Ordering::Relaxed);
        }
    });

    WIRE_SPY_ENABLED.load(Ordering::Relaxed)
}

fn wire_spy_log(direction: &str, msg: &Message) {
    if wire_spy_enabled() && !WIRE_SPY_BYTES_ONLY.load(Ordering::Relaxed) {
        eprintln!("[WIRE] {direction} {msg:?}");
    }
}

fn wire_spy_bytes(direction: &str, bytes: &[u8]) {
    if wire_spy_enabled() {
        eprintln!(
            "[WIRE] {direction} {} bytes: {:02x?}",
            bytes.len(),
            &bytes[..bytes.len().min(64)]
        );
    }
}

const RECV_BUF_COMPACT_THRESHOLD: usize = 64 * 1024;
const FRAME_LEN_PREFIX_SIZE: usize = 4;

fn compact_recv_buffer(buf: &mut Vec<u8>, unread_start: &mut usize) {
    if *unread_start == buf.len() {
        buf.clear();
        *unread_start = 0;
        return;
    }

    if *unread_start >= RECV_BUF_COMPACT_THRESHOLD && *unread_start >= buf.len() / 2 {
        buf.drain(..*unread_start);
        *unread_start = 0;
    }
}

fn advance_past_frame(buf: &mut Vec<u8>, unread_start: &mut usize, frame_end: usize) {
    *unread_start = frame_end;
    compact_recv_buffer(buf, unread_start);
}

/// Cached TypePlan for Message deserialization.
///
/// Building a TypePlan walks the entire type tree (enum variants, fields, etc.)
/// and allocates arena storage. Caching it here avoids rebuilding on every frame.
/// This is safe because `try_decode_one_from_buffer` is a non-generic function,
/// so the OnceLock static cannot be merged across monomorphizations.
static MESSAGE_TYPE_PLAN: OnceLock<facet_reflect::TypePlan<Message>> = OnceLock::new();

fn message_type_plan() -> &'static facet_reflect::TypePlan<Message> {
    MESSAGE_TYPE_PLAN
        .get_or_init(|| facet_reflect::TypePlan::<Message>::build().expect("TypePlan for Message"))
}

fn try_decode_one_from_buffer(
    buf: &mut Vec<u8>,
    unread_start: &mut usize,
    _scan_from: &mut usize,
    last_decoded: &mut Vec<u8>,
) -> io::Result<Option<Message>> {
    if *unread_start > buf.len() {
        *unread_start = buf.len();
    }

    let unread = &buf[*unread_start..];
    if unread.len() < FRAME_LEN_PREFIX_SIZE {
        return Ok(None);
    }

    let frame_len = u32::from_le_bytes([unread[0], unread[1], unread[2], unread[3]]) as usize;
    let frame_end = *unread_start + FRAME_LEN_PREFIX_SIZE + frame_len;
    if frame_end > buf.len() {
        return Ok(None);
    }

    let frame_start = *unread_start + FRAME_LEN_PREFIX_SIZE;
    let frame = &buf[frame_start..frame_end];

    wire_spy_bytes("<-- frame", frame);

    let plan = message_type_plan();
    let partial = plan
        .partial_owned()
        .map_err(|e| io::Error::other(format!("alloc: {e}")))?;
    let msg: Message = match facet_postcard::from_slice_into(frame, partial) {
        Ok(partial) => match partial.build() {
            Ok(heap_value) => match heap_value.materialize::<Message>() {
                Ok(msg) => msg,
                Err(e) => {
                    *last_decoded = frame.to_vec();
                    advance_past_frame(buf, unread_start, frame_end);
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("materialize: {e}"),
                    ));
                }
            },
            Err(e) => {
                *last_decoded = frame.to_vec();
                advance_past_frame(buf, unread_start, frame_end);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("build: {e}"),
                ));
            }
        },
        Err(e) => {
            *last_decoded = frame.to_vec();
            advance_past_frame(buf, unread_start, frame_end);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("postcard: {e}"),
            ));
        }
    };

    *last_decoded = frame.to_vec();
    wire_spy_log("<--", &msg);
    advance_past_frame(buf, unread_start, frame_end);
    Ok(Some(msg))
}

/// A length-prefixed async stream connection.
///
/// Handles encoding/decoding of roam messages over any async byte stream using
/// a 4-byte little-endian frame length prefix.
///
/// Generic over the transport type `S` which must implement `AsyncRead + AsyncWrite + Unpin`.
/// This allows the same framing logic to work with TCP sockets, Unix domain sockets,
/// or any other async byte stream.
pub struct LengthPrefixedFramed<S> {
    stream: S,
    buf: Vec<u8>,
    unread_start: usize,
    scan_from: usize,
    /// Last successfully decoded frame bytes (for error recovery/debugging).
    pub last_decoded: Vec<u8>,
    /// Buffer for encoding messages to avoid reallocations.
    encode_buf: Vec<u8>,
}

impl<S> LengthPrefixedFramed<S> {
    /// Create a new framed connection from an async stream.
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::new(),
            unread_start: 0,
            scan_from: 0,
            last_decoded: Vec::new(),
            encode_buf: Vec::with_capacity(1024),
        }
    }

    /// Get a reference to the underlying stream.
    pub fn stream(&self) -> &S {
        &self.stream
    }

    /// Get a mutable reference to the underlying stream.
    pub fn stream_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Consume the framed wrapper and return the underlying stream.
    pub fn into_inner(self) -> S {
        self.stream
    }
}

struct VecWriter<'a> {
    out: &'a mut Vec<u8>,
}

impl facet_postcard::Writer for VecWriter<'_> {
    fn write_byte(&mut self, byte: u8) -> Result<(), facet_postcard::SerializeError> {
        self.out.push(byte);
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), facet_postcard::SerializeError> {
        self.out.extend_from_slice(bytes);
        Ok(())
    }
}

impl<S> LengthPrefixedFramed<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Send a message over the connection.
    ///
    /// r[impl transport.bytestream.length-prefix] - Prefix each message with
    /// a 4-byte little-endian frame length.
    pub async fn send(&mut self, msg: &Message) -> io::Result<()> {
        wire_spy_log("-->", msg);

        self.encode_buf.clear();
        let mut writer = VecWriter {
            out: &mut self.encode_buf,
        };
        facet_postcard::to_writer_fallible(msg, &mut writer)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        let frame_len = u32::try_from(self.encode_buf.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "message too large for u32 length prefix",
            )
        })?;
        let header = frame_len.to_le_bytes();

        wire_spy_bytes("--> len", &header);
        wire_spy_bytes("-->", &self.encode_buf);
        self.stream.write_all(&header).await?;
        self.stream.write_all(&self.encode_buf).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Receive a message with a timeout.
    ///
    /// Returns `Ok(None)` if no message received within timeout or connection closed.
    pub async fn recv_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> io::Result<Option<Message>> {
        moire::time::timeout(timeout, self.recv_inner())
            .await
            .unwrap_or(Ok(None))
    }

    /// Receive a message (blocking until one arrives or connection closes).
    pub async fn recv(&mut self) -> io::Result<Option<Message>> {
        self.recv_inner().await
    }

    async fn recv_inner(&mut self) -> io::Result<Option<Message>> {
        loop {
            if let Some(msg) = try_decode_one_from_buffer(
                &mut self.buf,
                &mut self.unread_start,
                &mut self.scan_from,
                &mut self.last_decoded,
            )? {
                return Ok(Some(msg));
            }

            // Read more data
            let mut tmp = [0u8; 4096];
            let n = self.stream.read(&mut tmp).await?;
            if n == 0 {
                let trailing = self.buf.len().saturating_sub(self.unread_start);
                if wire_spy_enabled() {
                    eprintln!("[WIRE] <-- EOF (read 0 bytes)");
                }
                if trailing != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        format!("eof with {trailing} trailing bytes and no complete frame"),
                    ));
                }
                return Ok(None);
            }
            if wire_spy_enabled() {
                eprintln!("[WIRE] <-- read {} bytes: {:02x?}", n, &tmp[..n.min(64)]);
            }
            compact_recv_buffer(&mut self.buf, &mut self.unread_start);
            self.buf.extend_from_slice(&tmp[..n]);
        }
    }
}

impl<S> MessageTransport for LengthPrefixedFramed<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    async fn send(&mut self, msg: &Message) -> io::Result<()> {
        LengthPrefixedFramed::send(self, msg).await
    }

    async fn recv_timeout(&mut self, timeout: Duration) -> io::Result<Option<Message>> {
        LengthPrefixedFramed::recv_timeout(self, timeout).await
    }

    async fn recv(&mut self) -> io::Result<Option<Message>> {
        LengthPrefixedFramed::recv(self).await
    }

    fn last_decoded(&self) -> &[u8] {
        &self.last_decoded
    }
}
