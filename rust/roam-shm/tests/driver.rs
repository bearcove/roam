//! Integration tests for the SHM driver.
//!
//! These tests verify that roam RPC services can run over SHM transport,
//! including proper request/response handling and streaming.

#![cfg(feature = "tokio")]

use std::pin::Pin;

use roam_session::{ChannelRegistry, ServiceDispatcher, dispatch_call};
use roam_shm::driver::{establish_guest, establish_host_peer};
use roam_shm::guest::ShmGuest;
use roam_shm::host::ShmHost;
use roam_shm::layout::SegmentConfig;
use roam_shm::transport::ShmGuestTransport;

/// A simple echo service for testing.
#[derive(Clone)]
struct EchoService;

impl ServiceDispatcher for EchoService {
    fn dispatch(
        &self,
        method_id: u64,
        payload: Vec<u8>,
        request_id: u64,
        registry: &mut ChannelRegistry,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        match method_id {
            // Echo method: returns the input unchanged
            1 => dispatch_call::<String, String, (), _, _>(
                payload,
                request_id,
                registry,
                |input: String| async move { Ok(input) },
            ),
            // Add method: adds two numbers
            2 => dispatch_call::<(i32, i32), i32, (), _, _>(
                payload,
                request_id,
                registry,
                |(a, b): (i32, i32)| async move { Ok(a + b) },
            ),
            _ => roam_session::dispatch_unknown_method(request_id, registry),
        }
    }
}

fn create_host_and_guest() -> (ShmHost, ShmGuest) {
    let config = SegmentConfig::default();
    let host = ShmHost::create_heap(config).unwrap();
    let region = host.region();
    let guest = ShmGuest::attach(region).unwrap();
    (host, guest)
}

#[tokio::test]
async fn guest_calls_host_echo() {
    let (host, guest) = create_host_and_guest();
    let peer_id = guest.peer_id();

    // Set up guest-side driver (client)
    let guest_transport = ShmGuestTransport::new(guest);
    let (guest_handle, guest_driver) = establish_guest(guest_transport, EchoService);

    // Set up host-side driver (server)
    let (host_handle, host_driver) = establish_host_peer(host, peer_id, EchoService);

    // Spawn both drivers
    let guest_driver_handle = tokio::spawn(guest_driver.run());
    let host_driver_handle = tokio::spawn(host_driver.run());

    // Make an echo call from guest to host
    let input = "Hello, SHM!".to_string();
    let payload = facet_postcard::to_vec(&input).unwrap();

    let response = guest_handle.call_raw(1, payload).await.unwrap();

    // Response format: [0] = success marker, [1..] = serialized result
    assert_eq!(response[0], 0, "Expected success marker");
    let result: String = facet_postcard::from_slice(&response[1..]).unwrap();
    assert_eq!(result, input);

    // Clean shutdown - drop handles to close channels
    drop(guest_handle);
    drop(host_handle);

    // Wait for drivers to finish (they should exit when channels close)
    // Use a timeout to avoid hanging
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = guest_driver_handle.await;
        let _ = host_driver_handle.await;
    })
    .await;
}

#[tokio::test]
async fn guest_calls_host_add() {
    let (host, guest) = create_host_and_guest();
    let peer_id = guest.peer_id();

    let guest_transport = ShmGuestTransport::new(guest);
    let (guest_handle, guest_driver) = establish_guest(guest_transport, EchoService);
    let (_host_handle, host_driver) = establish_host_peer(host, peer_id, EchoService);

    tokio::spawn(guest_driver.run());
    tokio::spawn(host_driver.run());

    // Make an add call from guest to host
    let args = (17i32, 25i32);
    let payload = facet_postcard::to_vec(&args).unwrap();

    let response = guest_handle.call_raw(2, payload).await.unwrap();

    assert_eq!(response[0], 0, "Expected success marker");
    let result: i32 = facet_postcard::from_slice(&response[1..]).unwrap();
    assert_eq!(result, 42);
}

#[tokio::test]
async fn host_calls_guest() {
    let (host, guest) = create_host_and_guest();
    let peer_id = guest.peer_id();

    let guest_transport = ShmGuestTransport::new(guest);
    let (_guest_handle, guest_driver) = establish_guest(guest_transport, EchoService);
    let (host_handle, host_driver) = establish_host_peer(host, peer_id, EchoService);

    tokio::spawn(guest_driver.run());
    tokio::spawn(host_driver.run());

    // Make an echo call from host to guest
    let input = "Hello from host!".to_string();
    let payload = facet_postcard::to_vec(&input).unwrap();

    let response = host_handle.call_raw(1, payload).await.unwrap();

    assert_eq!(response[0], 0, "Expected success marker");
    let result: String = facet_postcard::from_slice(&response[1..]).unwrap();
    assert_eq!(result, input);
}

#[tokio::test]
async fn unknown_method_returns_error() {
    let (host, guest) = create_host_and_guest();
    let peer_id = guest.peer_id();

    let guest_transport = ShmGuestTransport::new(guest);
    let (guest_handle, guest_driver) = establish_guest(guest_transport, EchoService);
    let (_host_handle, host_driver) = establish_host_peer(host, peer_id, EchoService);

    tokio::spawn(guest_driver.run());
    tokio::spawn(host_driver.run());

    // Call unknown method
    let payload = facet_postcard::to_vec(&"test").unwrap();
    let response = guest_handle.call_raw(999, payload).await.unwrap();

    // Response format: [1] = error marker
    assert_eq!(response[0], 1, "Expected error marker");
}

#[tokio::test]
async fn multiple_sequential_calls() {
    let (host, guest) = create_host_and_guest();
    let peer_id = guest.peer_id();

    let guest_transport = ShmGuestTransport::new(guest);
    let (guest_handle, guest_driver) = establish_guest(guest_transport, EchoService);
    let (_host_handle, host_driver) = establish_host_peer(host, peer_id, EchoService);

    tokio::spawn(guest_driver.run());
    tokio::spawn(host_driver.run());

    // Make multiple calls sequentially
    for i in 0i32..10 {
        let args = (i, i * 2);
        let payload = facet_postcard::to_vec(&args).unwrap();
        let response = guest_handle.call_raw(2, payload).await.unwrap();
        assert_eq!(response[0], 0);
        let result: i32 = facet_postcard::from_slice(&response[1..]).unwrap();
        assert_eq!(result, i + i * 2);
    }
}
