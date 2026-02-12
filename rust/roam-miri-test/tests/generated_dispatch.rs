//! Test that generated dispatch code works correctly with multiple types under load.
//!
//! This tests the fix for the bug where ARGS_PLAN statics in generic functions
//! were incorrectly shared across different type instantiations, causing memory
//! corruption when deserializing.

use facet::Facet;
use roam_session::MessageTransport;
use roam_wire::Message;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;
use tokio::time::Duration;

// ============================================================================
// In-Memory Transport
// ============================================================================

struct InMemoryTransport {
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    last_decoded: Vec<u8>,
}

fn in_memory_transport_pair(buffer: usize) -> (InMemoryTransport, InMemoryTransport) {
    let (a_to_b_tx, a_to_b_rx) = mpsc::channel(buffer);
    let (b_to_a_tx, b_to_a_rx) = mpsc::channel(buffer);

    let a = InMemoryTransport {
        tx: a_to_b_tx,
        rx: b_to_a_rx,
        last_decoded: Vec::new(),
    };
    let b = InMemoryTransport {
        tx: b_to_a_tx,
        rx: a_to_b_rx,
        last_decoded: Vec::new(),
    };

    (a, b)
}

impl MessageTransport for InMemoryTransport {
    async fn send(&mut self, msg: &Message) -> io::Result<()> {
        self.tx
            .send(msg.clone())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "peer disconnected"))
    }

    async fn recv_timeout(&mut self, timeout_duration: Duration) -> io::Result<Option<Message>> {
        match tokio::time::timeout(timeout_duration, self.rx.recv()).await {
            Ok(msg) => Ok(msg),
            Err(_) => Ok(None),
        }
    }

    async fn recv(&mut self) -> io::Result<Option<Message>> {
        Ok(self.rx.recv().await)
    }

    fn last_decoded(&self) -> &[u8] {
        &self.last_decoded
    }
}

// ============================================================================
// Test Service
// ============================================================================

/// Service with multiple methods using different argument types.
/// This ensures each method gets its own ARGS_PLAN in generated code.
#[roam::service]
trait TestService {
    async fn handle_u64(&self, n: u64) -> u64;
    async fn handle_vec(&self, data: Vec<u8>) -> Vec<u8>;
    async fn handle_complex(&self, req: ComplexRequest) -> ComplexResponse;
}

#[derive(Debug, Clone, Facet)]
struct ComplexRequest {
    id: u64,
    data: Vec<u8>,
    tags: Vec<String>,
}

#[derive(Debug, Clone, Facet)]
struct ComplexResponse {
    request_id: u64,
    processed_bytes: usize,
    checksum: u64,
}

#[derive(Clone)]
struct TestServiceImpl {
    calls: Arc<AtomicUsize>,
}

impl TestService for TestServiceImpl {
    async fn handle_u64(&self, _cx: &roam_session::Context, n: u64) -> u64 {
        self.calls.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(10)).await;
        n
    }

    async fn handle_vec(&self, _cx: &roam_session::Context, data: Vec<u8>) -> Vec<u8> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let mut result = data;
        result.reverse();
        result
    }

    async fn handle_complex(
        &self,
        _cx: &roam_session::Context,
        req: ComplexRequest,
    ) -> ComplexResponse {
        self.calls.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let checksum = req.data.iter().map(|&b| b as u64).sum::<u64>();
        ComplexResponse {
            request_id: req.id,
            processed_bytes: req.data.len(),
            checksum,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_concurrent_mixed_types() {
    use roam_session::{HandshakeConfig, NoDispatcher, accept_framed, initiate_framed};

    // Create service
    let service_impl = TestServiceImpl {
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let dispatcher = TestServiceDispatcher::new(service_impl);

    // Create in-memory transport pair
    let (client_transport, server_transport) = in_memory_transport_pair(8192);

    // Establish connections
    let client_fut = initiate_framed(client_transport, HandshakeConfig::default(), NoDispatcher);
    let server_fut = accept_framed(server_transport, HandshakeConfig::default(), dispatcher);

    let (client_setup, server_setup) = tokio::try_join!(client_fut, server_fut).unwrap();

    let (client_handle, _incoming_client, client_driver) = client_setup;
    let (_server_handle, _incoming_server, server_driver) = server_setup;

    // Spawn drivers
    tokio::spawn(async move { client_driver.run().await });
    tokio::spawn(async move { server_driver.run().await });

    // Create client
    let client = TestServiceClient::new(client_handle);

    // Run concurrent mixed-type calls
    let mut handles = vec![];

    for i in 0..50 {
        let client = client.clone();
        let handle = tokio::spawn(async move {
            match i % 3 {
                0 => {
                    // u64 call
                    let result = client.handle_u64(42).await.unwrap();
                    assert_eq!(result, 42);
                }
                1 => {
                    // Vec<u8> call
                    let data = vec![1u8, 2, 3, 4, 5];
                    let result = client.handle_vec(data.clone()).await.unwrap();
                    assert_eq!(result, vec![5, 4, 3, 2, 1]);
                }
                2 => {
                    // Complex struct call
                    let req = ComplexRequest {
                        id: i as u64,
                        data: vec![10, 20, 30],
                        tags: vec!["test".to_string()],
                    };
                    let result = client.handle_complex(req).await.unwrap();
                    assert_eq!(result.checksum, 60);
                    assert_eq!(result.processed_bytes, 3);
                }
                _ => unreachable!(),
            }
        });
        handles.push(handle);
    }

    // Wait for all calls to complete
    for handle in handles {
        handle.await.unwrap();
    }

    eprintln!("✓ All concurrent mixed-type calls completed successfully");
}
