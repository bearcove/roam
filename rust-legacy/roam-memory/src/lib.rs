#![deny(unsafe_code)]

//! In-memory transport for roam message-level connections.
//!
//! This crate provides a bidirectional in-memory [`MemoryTransport`] pair that
//! implements [`roam_core::MessageTransport`]. It is useful for tests,
//! benchmarks, and embedding scenarios where no OS transport is needed.
//!
//! # Example
//!
//! ```ignore
//! use roam_memory::memory_transport_pair;
//! use roam_core::{HandshakeConfig, NoDispatcher, accept_framed, initiate_framed};
//!
//! let (client_transport, server_transport) = memory_transport_pair(256);
//!
//! let client_fut = initiate_framed(client_transport, HandshakeConfig::default(), NoDispatcher);
//! let server_fut = accept_framed(server_transport, HandshakeConfig::default(), NoDispatcher);
//! let _ = tokio::try_join!(client_fut, server_fut)?;
//! # Ok::<(), roam_core::ConnectionError>(())
//! ```

use std::io;
use std::time::Duration;

use roam_core::MessageTransport;
use roam_types::Message;
use tokio::sync::mpsc;

/// A message transport backed by in-process channels.
///
/// Create connected endpoints with [`memory_transport_pair`].
pub struct MemoryTransport {
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    last_decoded: Vec<u8>,
}

impl MemoryTransport {
    fn new(tx: mpsc::Sender<Message>, rx: mpsc::Receiver<Message>) -> Self {
        Self {
            tx,
            rx,
            last_decoded: Vec::new(),
        }
    }
}

/// Create a connected pair of in-memory transports.
///
/// `buffer` is the channel capacity for each direction.
pub fn memory_transport_pair(buffer: usize) -> (MemoryTransport, MemoryTransport) {
    let (a_to_b_tx, a_to_b_rx) = mpsc::channel(buffer);
    let (b_to_a_tx, b_to_a_rx) = mpsc::channel(buffer);

    let a = MemoryTransport::new(a_to_b_tx, b_to_a_rx);
    let b = MemoryTransport::new(b_to_a_tx, a_to_b_rx);
    (a, b)
}

impl MessageTransport for MemoryTransport {
    async fn send(&mut self, msg: &Message) -> io::Result<()> {
        self.tx
            .send(msg.clone())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "peer disconnected"))
    }

    async fn recv_timeout(&mut self, timeout: Duration) -> io::Result<Option<Message>> {
        Ok(moire::time::timeout(timeout, self.rx.recv())
            .await
            .unwrap_or(None))
    }

    async fn recv(&mut self) -> io::Result<Option<Message>> {
        Ok(self.rx.recv().await)
    }

    fn last_decoded(&self) -> &[u8] {
        &self.last_decoded
    }
}
