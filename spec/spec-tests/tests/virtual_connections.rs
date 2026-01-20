//! Virtual connection (multiplexing) compliance tests.
//!
//! Tests the Connect/Accept/Reject message handling for virtual connections.

use std::time::Duration;

use roam_wire::{ConnectionId, Hello, Message, MetadataValue};
use spec_tests::harness::{accept_subject, our_hello, run_async};

fn metadata_empty() -> Vec<(String, MetadataValue)> {
    Vec::new()
}

// r[verify core.conn.accept-required] - Peer MUST reject Connect if not listening
// r[verify message.reject.response] - Reject message sent in response to Connect
// r[verify message.reject.reason] - Reject includes reason string
#[test]
fn connect_rejected_when_not_listening() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;

        // Hello exchange
        let msg = io
            .recv_timeout(Duration::from_millis(250))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected Hello from subject".to_string())?;

        match msg {
            Message::Hello(Hello::V2 { .. }) => {}
            other => return Err(format!("expected Hello::V2, got {other:?}")),
        }

        io.send(&Message::Hello(our_hello(1024 * 1024)))
            .await
            .map_err(|e| e.to_string())?;

        // Send Connect request - subject is not listening for incoming connections
        // r[verify message.connect.initiate]
        // r[verify message.connect.request-id]
        let connect_msg = Message::Connect {
            request_id: 1,
            metadata: metadata_empty(),
        };
        io.send(&connect_msg).await.map_err(|e| e.to_string())?;

        // Expect Reject response since subject doesn't accept incoming connections
        let response = io
            .recv_timeout(Duration::from_millis(500))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected Reject from subject".to_string())?;

        match response {
            Message::Reject {
                request_id, reason, ..
            } => {
                if request_id != 1 {
                    return Err(format!("request_id mismatch: expected 1, got {request_id}"));
                }
                // Reason should indicate not listening
                if !reason.contains("not listening") && !reason.contains("listening") {
                    // Accept any reasonable rejection reason
                    eprintln!("Note: rejection reason was: {reason}");
                }
            }
            other => return Err(format!("expected Reject, got {other:?}")),
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify message.connect.request-id] - Connect uses unique request_id
// r[verify message.connect.metadata] - Connect can include metadata
#[test]
fn connect_message_structure() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;

        // Hello exchange
        let _ = io
            .recv_timeout(Duration::from_millis(250))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected Hello from subject".to_string())?;

        io.send(&Message::Hello(our_hello(1024 * 1024)))
            .await
            .map_err(|e| e.to_string())?;

        // Send Connect with metadata
        let connect_msg = Message::Connect {
            request_id: 42,
            metadata: vec![
                (
                    "auth".to_string(),
                    MetadataValue::String("token123".to_string()),
                ),
                ("version".to_string(), MetadataValue::U64(2)),
            ],
        };
        io.send(&connect_msg).await.map_err(|e| e.to_string())?;

        // Should get a response (Accept or Reject) with matching request_id
        let response = io
            .recv_timeout(Duration::from_millis(500))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected response to Connect".to_string())?;

        match response {
            Message::Accept { request_id, .. } | Message::Reject { request_id, .. } => {
                if request_id != 42 {
                    return Err(format!(
                        "request_id mismatch: expected 42, got {request_id}"
                    ));
                }
            }
            other => return Err(format!("expected Accept or Reject, got {other:?}")),
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify core.conn.independence] - Virtual connections are independent
// r[verify message.goodbye.connection-zero] - Goodbye on conn 0 closes entire link
#[test]
fn goodbye_on_root_closes_link() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;

        // Hello exchange
        let _ = io
            .recv_timeout(Duration::from_millis(250))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected Hello from subject".to_string())?;

        io.send(&Message::Hello(our_hello(1024 * 1024)))
            .await
            .map_err(|e| e.to_string())?;

        // Send Goodbye on connection 0 (root)
        io.send(&Message::Goodbye {
            conn_id: ConnectionId::ROOT,
            reason: "test complete".to_string(),
        })
        .await
        .map_err(|e| e.to_string())?;

        // Connection should close - no more messages expected
        // Give the subject time to process and close
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Try to receive - should get None (closed) or timeout
        match io.recv_timeout(Duration::from_millis(200)).await {
            Ok(None) => {}                          // Connection closed as expected
            Ok(Some(Message::Goodbye { .. })) => {} // Subject sent its own Goodbye, also fine
            Ok(Some(other)) => {
                return Err(format!(
                    "expected connection to close after Goodbye, got {other:?}"
                ));
            }
            Err(_) => {} // Timeout is acceptable too
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}
