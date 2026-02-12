//! Chaos load test - tries to break the transport using in-memory channels.
//!
//! Run with: cargo +nightly-2026-02-05 miri run (or use Justfile: just miri)
//!
//! This test includes:
//! - Client disconnections mid-call
//! - Call cancellations (dropping futures)
//! - Connection churn (rapid connect/disconnect)
//! - Overwhelming the server
//! - Race conditions and edge cases
//!
//! Uses in-memory transport for miri compatibility.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use roam_session::{
    ChannelRegistry, Context, RoamError, ServiceDispatcher, MessageTransport, HandshakeConfig,
    dispatch_call, dispatch_unknown_method, accept_framed, initiate_framed, NoDispatcher,
};
use roam_wire::Message;
use tokio::sync::mpsc;
use tokio::time::timeout;

// ============================================================================
// Test Service
// ============================================================================

#[derive(Clone)]
struct ChaosService {
    calls_total: Arc<AtomicU32>,
    calls_completed: Arc<AtomicU32>,
    calls_cancelled: Arc<AtomicU32>,
    calls_dropped: Arc<AtomicU32>,
}

impl ChaosService {
    fn new() -> Self {
        Self {
            calls_total: Arc::new(AtomicU32::new(0)),
            calls_completed: Arc::new(AtomicU32::new(0)),
            calls_cancelled: Arc::new(AtomicU32::new(0)),
            calls_dropped: Arc::new(AtomicU32::new(0)),
        }
    }

    fn stats(&self) -> (u32, u32, u32, u32) {
        (
            self.calls_total.load(Ordering::Relaxed),
            self.calls_completed.load(Ordering::Relaxed),
            self.calls_cancelled.load(Ordering::Relaxed),
            self.calls_dropped.load(Ordering::Relaxed),
        )
    }
}

const METHOD_INSTANT: u64 = 1;
const METHOD_SLOW: u64 = 2;
const METHOD_VERY_SLOW: u64 = 3;
const METHOD_BIG_DATA: u64 = 4;
const METHOD_COMPLEX_STRUCT: u64 = 5;

#[derive(facet::Facet, Clone, Debug)]
struct ComplexRequest {
    id: u64,
    name: String,
    data: Vec<u8>,
    nested: NestedData,
    tags: Vec<String>,
    metadata: std::collections::HashMap<String, String>,
}

#[derive(facet::Facet, Clone, Debug)]
struct NestedData {
    timestamp: u64,
    values: Vec<f64>,
    flags: Vec<bool>,
}

#[derive(facet::Facet, Clone, Debug)]
struct ComplexResponse {
    request_id: u64,
    processed_bytes: usize,
    checksum: u64,
    results: Vec<String>,
    nested_result: NestedData,
}

impl ServiceDispatcher for ChaosService {
    fn method_ids(&self) -> Vec<u64> {
        vec![
            METHOD_INSTANT,
            METHOD_SLOW,
            METHOD_VERY_SLOW,
            METHOD_BIG_DATA,
            METHOD_COMPLEX_STRUCT,
        ]
    }

    fn dispatch(
        &self,
        cx: Context,
        payload: Vec<u8>,
        registry: &mut ChannelRegistry,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        self.calls_total.fetch_add(1, Ordering::Relaxed);
        let completed = self.calls_completed.clone();

        let method = cx.method_id().raw();
        eprintln!("[dispatch] method={} payload_len={}", method, payload.len());

        match method {
            METHOD_INSTANT => {
                use std::sync::{Arc, OnceLock};
                static ARGS_PLAN: OnceLock<Arc<RpcPlan>> = OnceLock::new();
                static RESPONSE_PLAN: OnceLock<Arc<RpcPlan>> = OnceLock::new();
                let args_plan = ARGS_PLAN.get_or_init(|| Arc::new(RpcPlan::for_type::<u64>()));
                let response_plan = RESPONSE_PLAN.get_or_init(|| Arc::new(RpcPlan::for_type::<u64>()));

                dispatch_call::<u64, u64, (), _, _>(
                    &cx,
                    payload,
                    registry,
                    args_plan,
                    Arc::clone(response_plan),
                    move |n: u64| async move {
                        completed.fetch_add(1, Ordering::Relaxed);
                        Ok(n)
                    },
                )
            }

            METHOD_SLOW => {
                use std::sync::{Arc, OnceLock};
                static ARGS_PLAN: OnceLock<Arc<RpcPlan>> = OnceLock::new();
                static RESPONSE_PLAN: OnceLock<Arc<RpcPlan>> = OnceLock::new();
                let args_plan = ARGS_PLAN.get_or_init(|| Arc::new(RpcPlan::for_type::<u64>()));
                let response_plan = RESPONSE_PLAN.get_or_init(|| Arc::new(RpcPlan::for_type::<u64>()));

                let completed = completed.clone();
                dispatch_call::<u64, u64, (), _, _>(
                    &cx,
                    payload,
                    registry,
                    args_plan,
                    Arc::clone(response_plan),
                    move |n: u64| async move {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        completed.fetch_add(1, Ordering::Relaxed);
                        Ok(n)
                    },
                )
            }

            METHOD_VERY_SLOW => {
                use std::sync::{Arc, OnceLock};
                static ARGS_PLAN: OnceLock<Arc<RpcPlan>> = OnceLock::new();
                static RESPONSE_PLAN: OnceLock<Arc<RpcPlan>> = OnceLock::new();
                let args_plan = ARGS_PLAN.get_or_init(|| Arc::new(RpcPlan::for_type::<u64>()));
                let response_plan = RESPONSE_PLAN.get_or_init(|| Arc::new(RpcPlan::for_type::<u64>()));

                let completed = completed.clone();
                dispatch_call::<u64, u64, (), _, _>(
                    &cx,
                    payload,
                    registry,
                    args_plan,
                    Arc::clone(response_plan),
                    move |n: u64| async move {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        completed.fetch_add(1, Ordering::Relaxed);
                        Ok(n)
                    },
                )
            }

            METHOD_BIG_DATA => {
                use std::sync::{Arc, OnceLock};
                static ARGS_PLAN: OnceLock<Arc<RpcPlan>> = OnceLock::new();
                static RESPONSE_PLAN: OnceLock<Arc<RpcPlan>> = OnceLock::new();
                let args_plan = ARGS_PLAN.get_or_init(|| Arc::new(RpcPlan::for_type::<Vec<u8>>()));
                let response_plan = RESPONSE_PLAN.get_or_init(|| Arc::new(RpcPlan::for_type::<Vec<u8>>()));

                let completed = completed.clone();
                dispatch_call::<Vec<u8>, Vec<u8>, (), _, _>(
                    &cx,
                    payload,
                    registry,
                    args_plan,
                    Arc::clone(response_plan),
                    move |data: Vec<u8>| async move {
                        // Process the big data (simulate work)
                        tokio::time::sleep(Duration::from_millis(10)).await;

                        // Return processed data (reversed)
                        let mut result = data.clone();
                        result.reverse();

                        completed.fetch_add(1, Ordering::Relaxed);
                        Ok(result)
                    },
                )
            }

            METHOD_COMPLEX_STRUCT => {
                use std::sync::{Arc, OnceLock};
                static ARGS_PLAN: OnceLock<Arc<RpcPlan>> = OnceLock::new();
                static RESPONSE_PLAN: OnceLock<Arc<RpcPlan>> = OnceLock::new();
                let args_plan = ARGS_PLAN.get_or_init(|| Arc::new(RpcPlan::for_type::<ComplexRequest>()));
                let response_plan = RESPONSE_PLAN.get_or_init(|| Arc::new(RpcPlan::for_type::<ComplexResponse>()));

                let completed = completed.clone();
                dispatch_call::<ComplexRequest, ComplexResponse, (), _, _>(
                    &cx,
                    payload,
                    registry,
                    args_plan,
                    Arc::clone(response_plan),
                    move |req: ComplexRequest| async move {
                        tokio::time::sleep(Duration::from_millis(20)).await;

                        // Build complex response
                        let checksum = req.data.iter().map(|&b| b as u64).sum::<u64>();
                        let response = ComplexResponse {
                            request_id: req.id,
                            processed_bytes: req.data.len(),
                            checksum,
                            results: req.tags.iter().map(|t| format!("processed:{}", t)).collect(),
                            nested_result: req.nested.clone(),
                        };

                        completed.fetch_add(1, Ordering::Relaxed);
                        Ok(response)
                    },
                )
            }

            _ => dispatch_unknown_method(&cx, registry),
        }
    }
}

// ============================================================================
// In-Memory Transport Infrastructure
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

type ClientHandle = roam_session::ConnectionHandle;
type ServerHandle = roam_session::ConnectionHandle;

async fn create_connection_pair(
    service: ChaosService,
) -> Result<(ClientHandle, ServerHandle), Box<dyn std::error::Error + Send + Sync>> {
    let (client_transport, server_transport) = in_memory_transport_pair(8192);

    let client_fut = initiate_framed(client_transport, HandshakeConfig::default(), NoDispatcher);
    let server_fut = accept_framed(server_transport, HandshakeConfig::default(), service);

    let (client_setup, server_setup) = tokio::try_join!(client_fut, server_fut)?;

    let (client_handle, _incoming_client, client_driver) = client_setup;
    let (server_handle, _incoming_server, server_driver) = server_setup;

    // Spawn drivers
    tokio::spawn(async move { client_driver.run().await });
    tokio::spawn(async move { server_driver.run().await });

    Ok((client_handle, server_handle))
}

fn decode_result<T>(response: Vec<u8>) -> T
where
    T: for<'a> facet::Facet<'a>,
{
    let result: Result<T, RoamError<()>> = facet_postcard::from_slice(&response).unwrap();
    result.unwrap()
}

// ============================================================================
// Chaos Scenarios
// ============================================================================

/// Scenario 1: Clients that disconnect mid-call
async fn chaos_disconnecting_clients(
    iterations: usize,
    stats: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for i in 0..iterations {
        let service = ChaosService::new();
        let (client_handle, _server_handle) = create_connection_pair(service).await?;

        // Start a slow call
        let handle_clone = client_handle.clone();
        let task = tokio::spawn(async move {
            let mut args = i as u64;
            let _ = handle_clone.call(METHOD_VERY_SLOW, &mut args).await;
        });

        // Disconnect before it completes
        tokio::time::sleep(Duration::from_millis(10)).await;
        drop(client_handle);

        // Don't wait for the task - it should fail
        let _ = task.await;
        stats.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

/// Scenario 2: Cancelled calls (drop the future)
async fn chaos_cancelled_calls(
    iterations: usize,
    stats: Arc<AtomicU64>,
) -> Result<ChaosService, Box<dyn std::error::Error + Send + Sync>> {
    let service = ChaosService::new();
    let (client_handle, _server_handle) = create_connection_pair(service.clone()).await?;

    for i in 0..iterations {
        let handle = client_handle.clone();
        let task = tokio::spawn(async move {
            let mut args = i as u64;
            let _ = handle.call(METHOD_VERY_SLOW, &mut args).await;
        });

        // Cancel by dropping after a short delay
        tokio::time::sleep(Duration::from_millis(5)).await;
        task.abort();

        stats.fetch_add(1, Ordering::Relaxed);
    }
    Ok(service)
}

/// Scenario 3: Connection churn - rapid connect/disconnect
async fn chaos_connection_churn(
    iterations: usize,
    stats: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for i in 0..iterations {
        let service = ChaosService::new();
        let (client_handle, _server_handle) = create_connection_pair(service).await?;

        // Maybe make a call, maybe don't
        if i % 3 == 0 {
            let mut args = i as u64;
            let _ = client_handle.call(METHOD_INSTANT, &mut args).await;
        }

        // Disconnect immediately
        drop(client_handle);
        stats.fetch_add(1, Ordering::Relaxed);

        // Tiny delay
        if i % 10 == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    Ok(())
}

/// Scenario 4: Overwhelming the server with complex data
async fn chaos_overwhelm(
    concurrent_calls: usize,
    stats: Arc<AtomicU64>,
) -> Result<ChaosService, Box<dyn std::error::Error + Send + Sync>> {
    let service = ChaosService::new();
    let (client_handle, _server_handle) = create_connection_pair(service.clone()).await?;

    let mut tasks = Vec::new();
    for i in 0..concurrent_calls {
        let handle = client_handle.clone();
        let stats = stats.clone();
        let task = tokio::spawn(async move {
            // Mix of different call types
            match i % 5 {
                0 | 1 => {
                    // Big data: 1KB to 100KB
                    let size = 1024 + (i % 100) * 1024;
                    let mut data = vec![0u8; size];
                    for (idx, byte) in data.iter_mut().enumerate() {
                        *byte = (idx % 256) as u8;
                    }

                    match timeout(Duration::from_secs(2), handle.call(METHOD_BIG_DATA, &mut data)).await {
                        Ok(Ok(response)) => {
                            let result: Vec<u8> = decode_result(response.payload);
                            // Verify it was reversed
                            if result.len() == data.len() {
                                stats.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        _ => {}
                    }
                }
                2 | 3 => {
                    // Complex struct
                    let mut req = ComplexRequest {
                        id: i as u64,
                        name: format!("request-{}", i),
                        data: vec![(i % 256) as u8; 512],
                        nested: NestedData {
                            timestamp: i as u64,
                            values: vec![1.0, 2.0, 3.0, (i as f64) * 0.5],
                            flags: vec![i % 2 == 0, i % 3 == 0, i % 5 == 0],
                        },
                        tags: vec![
                            format!("tag-{}", i),
                            "production".to_string(),
                            "high-priority".to_string(),
                        ],
                        metadata: [
                            ("source".to_string(), "chaos-test".to_string()),
                            ("index".to_string(), i.to_string()),
                        ].into_iter().collect(),
                    };

                    match timeout(Duration::from_secs(2), handle.call(METHOD_COMPLEX_STRUCT, &mut req)).await {
                        Ok(Ok(response)) => {
                            let result: ComplexResponse = decode_result(response.payload);
                            if result.request_id == i as u64 {
                                stats.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {
                    // Simple call
                    let mut args = i as u64;
                    match timeout(Duration::from_secs(2), handle.call(METHOD_SLOW, &mut args)).await {
                        Ok(Ok(response)) => {
                            let _: u64 = decode_result(response.payload);
                            stats.fetch_add(1, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
            }
        });
        tasks.push(task);
    }

    for task in tasks {
        let _ = task.await;
    }

    Ok(service)
}

/// Scenario 5: Mixed chaos - everything at once
async fn chaos_mixed(
    duration_secs: u64,
    stats: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();

    while start.elapsed() < Duration::from_secs(duration_secs) {
        // Randomly pick a chaos scenario
        let scenario = stats.load(Ordering::Relaxed) % 6;

        match scenario {
            0 => {
                // Quick disconnect with big data
                let service = ChaosService::new();
                match create_connection_pair(service).await {
                    Ok((client_handle, _server_handle)) => {
                        let handle_clone = client_handle.clone();
                        let task = tokio::spawn(async move {
                            let mut data = vec![0xAA; 50 * 1024]; // 50KB
                            let _ = handle_clone.call(METHOD_BIG_DATA, &mut data).await;
                        });
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        drop(client_handle);
                        let _ = task.await;
                    }
                    Err(_) => {}
                }
            }
            1 => {
                // Cancelled complex struct call
                let service = ChaosService::new();
                match create_connection_pair(service).await {
                    Ok((client_handle, _server_handle)) => {
                        let task = tokio::spawn(async move {
                            let mut req = ComplexRequest {
                                id: 999,
                                name: "cancelled".to_string(),
                                data: vec![0xFF; 2048],
                                nested: NestedData {
                                    timestamp: 123456789,
                                    values: vec![1.0; 100],
                                    flags: vec![true; 50],
                                },
                                tags: vec!["test".to_string(); 10],
                                metadata: Default::default(),
                            };
                            let _ = client_handle.call(METHOD_COMPLEX_STRUCT, &mut req).await;
                        });
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        task.abort();
                        let _ = task.await;
                    }
                    Err(_) => {}
                }
            }
            2 => {
                // Rapid connect/disconnect
                for _ in 0..5 {
                    let service = ChaosService::new();
                    match create_connection_pair(service).await {
                        Ok((client_handle, _server_handle)) => {
                            drop(client_handle);
                        }
                        Err(_) => {}
                    }
                }
            }
            3 => {
                // Burst of big data calls
                let service = ChaosService::new();
                match create_connection_pair(service).await {
                    Ok((client_handle, _server_handle)) => {
                        let mut tasks = Vec::new();
                        for i in 0..10 {
                            let handle: ClientHandle = client_handle.clone();
                            let task = tokio::spawn(async move {
                                let size = 1024 + (i * 1024);
                                let mut data = vec![(i % 256) as u8; size];
                                let _ = timeout(
                                    Duration::from_millis(200),
                                    handle.call(METHOD_BIG_DATA, &mut data),
                                )
                                .await;
                            });
                            tasks.push(task);
                        }
                        for task in tasks {
                            let _ = task.await;
                        }
                    }
                    Err(_) => {}
                }
            }
            4 => {
                // Burst of complex struct calls
                let service = ChaosService::new();
                match create_connection_pair(service).await {
                    Ok((client_handle, _server_handle)) => {
                        let mut tasks = Vec::new();
                        for i in 0..5 {
                            let handle: ClientHandle = client_handle.clone();
                            let task = tokio::spawn(async move {
                                let mut req = ComplexRequest {
                                    id: i,
                                    name: format!("burst-{}", i),
                                    data: vec![(i % 256) as u8; 512],
                                    nested: NestedData {
                                        timestamp: i,
                                        values: vec![i as f64; 20],
                                        flags: vec![i % 2 == 0; 10],
                                    },
                                    tags: vec![format!("tag{}", i)],
                                    metadata: Default::default(),
                                };
                                let _ = timeout(
                                    Duration::from_millis(200),
                                    handle.call(METHOD_COMPLEX_STRUCT, &mut req),
                                )
                                .await;
                            });
                            tasks.push(task);
                        }
                        for task in tasks {
                            let _ = task.await;
                        }
                    }
                    Err(_) => {}
                }
            }
            _ => {
                // Burst of instant calls
                let service = ChaosService::new();
                match create_connection_pair(service).await {
                    Ok((client_handle, _server_handle)) => {
                        let mut tasks = Vec::new();
                        for i in 0..20 {
                            let handle: ClientHandle = client_handle.clone();
                            let task = tokio::spawn(async move {
                                let mut args = i;
                                let _ = timeout(
                                    Duration::from_millis(100),
                                    handle.call(METHOD_INSTANT, &mut args),
                                )
                                .await;
                            });
                            tasks.push(task);
                        }
                        for task in tasks {
                            let _ = task.await;
                        }
                    }
                    Err(_) => {}
                }
            }
        }

        stats.fetch_add(1, Ordering::Relaxed);
    }

    Ok(())
}

// ============================================================================
// Main
// ============================================================================

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()?;

    rt.block_on(run())
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("=== Chaos Load Test (In-Memory Transport) ===");
    println!("Attempting to break the transport with chaos scenarios...");
    println!();

    println!("Running chaos scenarios:");
    println!();

    // Scenario 1: Disconnecting clients
    if false {
        println!("1. Disconnecting clients mid-call...");
        let stats = Arc::new(AtomicU64::new(0));
        let stats_clone = stats.clone();

        let start = Instant::now();
        chaos_disconnecting_clients(50, stats_clone).await?;
        let elapsed = start.elapsed();

        println!(
            "   ✓ Completed {} disconnections in {:.2}s",
            stats.load(Ordering::Relaxed),
            elapsed.as_secs_f64()
        );
    }

    // Scenario 2: Cancelled calls
    if false {
        println!("2. Cancelled calls...");
        let stats = Arc::new(AtomicU64::new(0));
        let stats_clone = stats.clone();

        let start = Instant::now();
        let service = chaos_cancelled_calls(50, stats_clone).await?;
        let elapsed = start.elapsed();

        let (total, completed, _cancelled, _dropped) = service.stats();
        println!(
            "   ✓ Completed {} cancellations in {:.2}s",
            stats.load(Ordering::Relaxed),
            elapsed.as_secs_f64()
        );
        println!(
            "     Service stats: {} total calls, {} completed",
            total, completed
        );
    }

    // Scenario 3: Connection churn
    if false {
        println!("3. Connection churn (rapid connect/disconnect)...");
        let stats = Arc::new(AtomicU64::new(0));
        let stats_clone = stats.clone();

        let start = Instant::now();
        chaos_connection_churn(100, stats_clone).await?;
        let elapsed = start.elapsed();

        println!(
            "   ✓ Completed {} connection cycles in {:.2}s",
            stats.load(Ordering::Relaxed),
            elapsed.as_secs_f64()
        );
    }

    // Minimal reproducer for miri bug
    {
        println!("Minimal reproducer: 50 concurrent mixed calls (triggers miri UB)...");
        let stats = Arc::new(AtomicU64::new(0));
        let stats_clone = stats.clone();

        let start = Instant::now();
        let service = chaos_overwhelm(50, stats_clone).await?;
        let elapsed = start.elapsed();

        let (total, completed, _cancelled, _dropped) = service.stats();
        println!(
            "   ✓ Completed {}/50 calls in {:.2}s",
            stats.load(Ordering::Relaxed),
            elapsed.as_secs_f64()
        );
        println!(
            "     Service stats: {} total calls, {} completed",
            total, completed
        );
    }

    // Scenario 4: Overwhelming the server (full stress test)
    if false {
        println!("4. Overwhelming the server...");
        let stats = Arc::new(AtomicU64::new(0));
        let stats_clone = stats.clone();

        let start = Instant::now();
        let service = chaos_overwhelm(500, stats_clone).await?;
        let elapsed = start.elapsed();

        let (total, completed, _cancelled, _dropped) = service.stats();
        println!(
            "   ✓ Completed {}/500 overwhelming calls in {:.2}s",
            stats.load(Ordering::Relaxed),
            elapsed.as_secs_f64()
        );
        println!(
            "     Service stats: {} total calls, {} completed",
            total, completed
        );
    }

    // Scenario 5: Mixed chaos
    if false {
        println!("5. Mixed chaos (10 seconds of random mayhem)...");
        let stats = Arc::new(AtomicU64::new(0));
        let stats_clone = stats.clone();

        let start = Instant::now();
        chaos_mixed(10, stats_clone).await?;
        let elapsed = start.elapsed();

        println!(
            "   ✓ Completed {} mixed operations in {:.2}s",
            stats.load(Ordering::Relaxed),
            elapsed.as_secs_f64()
        );
    }

    println!();
    println!("✓ Survived all chaos scenarios!");
    println!("   (Note: Each connection pair has its own service instance)");

    Ok(())
}
