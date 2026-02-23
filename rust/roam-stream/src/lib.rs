//! Byte-stream transport for roam.
//!
//! Implements [`Link`](roam_types::Link) over any `AsyncRead + AsyncWrite`
//! pair (TCP, Unix sockets, stdio) using 4-byte little-endian length-prefix
//! framing.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use roam_types::{Backing, Link, LinkRx, LinkTx, LinkTxPermit, WriteSlot};

/// A [`Link`](roam_types::Link) over a byte stream with length-prefix framing.
///
/// Wraps an `AsyncRead + AsyncWrite` pair. Each message is framed as
/// `[len: u32 LE][payload bytes]`.
// r[impl transport.stream]
// r[impl transport.stream.kinds]
pub struct StreamLink<R, W> {
    reader: R,
    writer: W,
}

impl<R, W> StreamLink<R, W> {
    /// Construct from separate read and write halves.
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl StreamLink<tokio::net::tcp::OwnedReadHalf, tokio::net::tcp::OwnedWriteHalf> {
    /// Wrap a [`TcpStream`](tokio::net::TcpStream).
    pub fn tcp(stream: tokio::net::TcpStream) -> Self {
        let (r, w) = stream.into_split();
        Self::new(r, w)
    }
}

impl StreamLink<tokio::io::Stdin, tokio::io::Stdout> {
    /// Wrap stdio (stdin for reading, stdout for writing).
    pub fn stdio() -> Self {
        Self::new(tokio::io::stdin(), tokio::io::stdout())
    }
}

#[cfg(unix)]
impl StreamLink<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf> {
    /// Wrap a [`UnixStream`](tokio::net::UnixStream).
    pub fn unix(stream: tokio::net::UnixStream) -> Self {
        let (r, w) = stream.into_split();
        Self::new(r, w)
    }
}

#[cfg(windows)]
impl
    StreamLink<
        tokio::io::ReadHalf<tokio::net::windows::named_pipe::NamedPipeClient>,
        tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeClient>,
    >
{
    /// Wrap a Windows named pipe client.
    pub fn named_pipe_client(pipe: tokio::net::windows::named_pipe::NamedPipeClient) -> Self {
        let (r, w) = tokio::io::split(pipe);
        Self::new(r, w)
    }
}

impl<R, W> Link for StreamLink<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Tx = StreamLinkTx;
    type Rx = StreamLinkRx<R>;

    fn split(self) -> (Self::Tx, Self::Rx) {
        let (tx_chan, mut rx_chan) = mpsc::channel::<Vec<u8>>(1);
        let mut writer = self.writer;

        let writer_task = tokio::spawn(async move {
            while let Some(bytes) = rx_chan.recv().await {
                writer
                    .write_all(&(bytes.len() as u32).to_le_bytes())
                    .await?;
                writer.write_all(&bytes).await?;
                writer.flush().await?;
            }
            writer.shutdown().await?;
            Ok(())
        });

        (
            StreamLinkTx {
                tx: tx_chan,
                writer_task,
            },
            StreamLinkRx {
                reader: self.reader,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Tx
// ---------------------------------------------------------------------------

/// Sending half of a [`StreamLink`].
///
/// Internally uses a bounded mpsc channel (capacity 1) to serialize writes
/// and provide backpressure. A background task drains the channel and writes
/// length-prefixed frames to the underlying stream.
pub struct StreamLinkTx {
    tx: mpsc::Sender<Vec<u8>>,
    writer_task: JoinHandle<io::Result<()>>,
}

/// Permit for sending one payload through a [`StreamLinkTx`].
pub struct StreamLinkTxPermit {
    permit: mpsc::OwnedPermit<Vec<u8>>,
}

/// Write slot for [`StreamLinkTx`].
pub struct StreamWriteSlot {
    buf: Vec<u8>,
    permit: mpsc::OwnedPermit<Vec<u8>>,
}

impl LinkTx for StreamLinkTx {
    type Permit = StreamLinkTxPermit;

    async fn reserve(&self) -> io::Result<Self::Permit> {
        let permit = self.tx.clone().reserve_owned().await.map_err(|_| {
            io::Error::new(io::ErrorKind::ConnectionReset, "stream writer task stopped")
        })?;
        Ok(StreamLinkTxPermit { permit })
    }

    async fn close(self) -> io::Result<()> {
        drop(self.tx);
        self.writer_task
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
    }
}

impl LinkTxPermit for StreamLinkTxPermit {
    type Slot = StreamWriteSlot;

    fn alloc(self, len: usize) -> io::Result<Self::Slot> {
        Ok(StreamWriteSlot {
            buf: vec![0u8; len],
            permit: self.permit,
        })
    }
}

impl WriteSlot for StreamWriteSlot {
    fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf
    }

    fn commit(self) {
        drop(self.permit.send(self.buf));
    }
}

// ---------------------------------------------------------------------------
// Rx
// ---------------------------------------------------------------------------

/// Receiving half of a [`StreamLink`].
pub struct StreamLinkRx<R> {
    reader: R,
}

impl<R: AsyncRead + Send + Unpin + 'static> LinkRx for StreamLinkRx<R> {
    type Error = io::Error;

    async fn recv(&mut self) -> io::Result<Option<Backing>> {
        let mut len_buf = [0u8; 4];
        match self.reader.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        self.reader.read_exact(&mut buf).await?;
        Ok(Some(Backing::Boxed(buf.into_boxed_slice())))
    }
}

// ---------------------------------------------------------------------------
// LocalLink
// ---------------------------------------------------------------------------

type BoxReader = Box<dyn AsyncRead + Send + Unpin>;
type BoxWriter = Box<dyn AsyncWrite + Send + Unpin>;

/// Platform-agnostic local IPC link.
///
/// Uses Unix domain sockets on Linux/macOS, named pipes on Windows.
/// Addresses are strings: a socket path on Unix, a named pipe path on Windows
/// (e.g. `\\.\pipe\my-service`).
// r[impl transport.stream.local]
pub struct LocalLink {
    inner: StreamLink<BoxReader, BoxWriter>,
}

impl LocalLink {
    /// Connect to a local endpoint by address.
    #[cfg(unix)]
    pub async fn connect(addr: &str) -> io::Result<Self> {
        let stream = tokio::net::UnixStream::connect(addr).await?;
        let (r, w) = stream.into_split();
        Ok(Self {
            inner: StreamLink::new(Box::new(r), Box::new(w)),
        })
    }

    /// Connect to a local endpoint by address.
    #[cfg(windows)]
    pub async fn connect(addr: &str) -> io::Result<Self> {
        let pipe = tokio::net::windows::named_pipe::ClientOptions::new().open(addr)?;
        let (r, w) = tokio::io::split(pipe);
        Ok(Self {
            inner: StreamLink::new(Box::new(r), Box::new(w)),
        })
    }
}

impl Link for LocalLink {
    type Tx = StreamLinkTx;
    type Rx = StreamLinkRx<BoxReader>;

    fn split(self) -> (Self::Tx, Self::Rx) {
        self.inner.split()
    }
}

// ---------------------------------------------------------------------------
// LocalLinkAcceptor
// ---------------------------------------------------------------------------

/// Accepts incoming [`LocalLink`] connections.
// r[impl transport.stream.local]
pub struct LocalLinkAcceptor {
    #[cfg(unix)]
    listener: tokio::net::UnixListener,
    /// On Windows, named pipes don't have a persistent listener object — each
    /// server instance accepts exactly one connection. We keep the current
    /// pending instance here, protected by a Mutex so `accept` can take `&self`.
    #[cfg(windows)]
    pending: tokio::sync::Mutex<tokio::net::windows::named_pipe::NamedPipeServer>,
}

impl LocalLinkAcceptor {
    /// Bind to a local address.
    #[cfg(unix)]
    pub fn bind(addr: impl Into<String>) -> io::Result<Self> {
        let listener = tokio::net::UnixListener::bind(addr.into())?;
        Ok(Self { listener })
    }

    /// Bind to a local address (named pipe path).
    #[cfg(windows)]
    pub fn bind(addr: impl Into<String>) -> io::Result<Self> {
        use tokio::net::windows::named_pipe::ServerOptions;
        let addr = addr.into();
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&addr)?;
        Ok(Self {
            addr,
            pending: tokio::sync::Mutex::new(server),
        })
    }

    /// Accept the next incoming connection.
    #[cfg(unix)]
    pub async fn accept(&self) -> io::Result<LocalLink> {
        let (stream, _addr) = self.listener.accept().await?;
        let (r, w) = stream.into_split();
        Ok(LocalLink {
            inner: StreamLink::new(Box::new(r), Box::new(w)),
        })
    }

    /// Accept the next incoming connection.
    #[cfg(windows)]
    pub async fn accept(&self) -> io::Result<LocalLink> {
        use tokio::net::windows::named_pipe::ServerOptions;
        let mut guard = self.pending.lock().await;
        guard.connect().await?;
        let next = ServerOptions::new().create(&self.addr)?;
        let connected = std::mem::replace(&mut *guard, next);
        drop(guard);
        let (r, w) = tokio::io::split(connected);
        Ok(LocalLink {
            inner: StreamLink::new(Box::new(r), Box::new(w)),
        })
    }
}
