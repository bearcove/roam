//! Streaming RPC compliance tests.
//!
//! Tests the streaming methods from the `Testbed` service:
//! - `sum(numbers: Rx<i32>) -> i64` - client-to-server streaming
//! - `generate(count: u32, output: Tx<i32>)` - server-to-client streaming
//! - `transform(input: Rx<String>, output: Tx<String>)` - bidirectional streaming

use std::time::Duration;

use facet::Facet;
use roam_hash::method_id_from_detail;
use roam_schema::{ArgDetail, MethodDetail};
use roam_wire::{Hello, Message, MetadataValue};
use spec_tests::harness::{accept_subject, our_hello, run_async};

// TODO: Remove this shim once facet implements `Facet` for `core::convert::Infallible`
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
struct Never;

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
enum RoamError<E> {
    User(E) = 0,
    UnknownMethod = 1,
    InvalidPayload = 2,
    Cancelled = 3,
}

fn testbed_method_id(
    method_name: &str,
    args: Vec<ArgDetail>,
    return_type: &'static facet::Shape,
) -> u64 {
    let detail = MethodDetail {
        service_name: "Testbed".into(),
        method_name: String::from(method_name).into(),
        args,
        return_type,
        doc: None,
    };
    method_id_from_detail(&detail)
}

fn metadata_empty() -> Vec<(String, MetadataValue)> {
    Vec::new()
}

/// Helper to do hello exchange.
async fn hello_exchange(io: &mut spec_tests::harness::CobsFramed) -> Result<(), String> {
    // Subject sends Hello first.
    let msg = io
        .recv_timeout(Duration::from_millis(250))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "expected Hello from subject".to_string())?;
    if !matches!(msg, Message::Hello(Hello::V1 { .. })) {
        return Err(format!("first message must be Hello, got {msg:?}"));
    }

    io.send(&Message::Hello(our_hello(1024 * 1024)))
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

// r[verify streaming.client-to-server] - Client pushes data, server aggregates
// r[verify streaming.data] - Data messages carry stream payloads
// r[verify streaming.close] - Close terminates stream gracefully
// r[verify streaming.id.parity] - Client uses odd stream IDs (initiator)
#[test]
fn streaming_sum_client_to_server() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;
        hello_exchange(&mut io).await?;

        // Get the method ID for `sum(numbers: Rx<i32>) -> i64`
        // Rx<i32> serializes as u64 on the wire (stream ID)
        let method_id = testbed_method_id(
            "sum",
            vec![ArgDetail {
                name: "numbers".into(),
                ty: <u64 as Facet>::SHAPE, // Rx<i32> is proxy'd to u64
            }],
            <i64 as Facet>::SHAPE,
        );

        // Allocate stream ID (odd = initiator)
        let stream_id: u64 = 1;

        // Send Request with stream ID as the payload
        // Payload: tuple of (stream_id: u64)
        let req_payload =
            facet_postcard::to_vec(&(stream_id,)).map_err(|e| format!("postcard args: {e}"))?;
        let req = Message::Request {
            request_id: 1,
            method_id,
            metadata: metadata_empty(),
            payload: req_payload,
        };
        io.send(&req).await.map_err(|e| e.to_string())?;

        // Send Data messages with numbers
        for n in [1i32, 2, 3, 4, 5] {
            let data_payload =
                facet_postcard::to_vec(&n).map_err(|e| format!("postcard data: {e}"))?;
            io.send(&Message::Data {
                stream_id,
                payload: data_payload,
            })
            .await
            .map_err(|e| e.to_string())?;
        }

        // Send Close to end the stream
        io.send(&Message::Close { stream_id })
            .await
            .map_err(|e| e.to_string())?;

        // Wait for Response
        let resp = io
            .recv_timeout(Duration::from_millis(500))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected Response from subject".to_string())?;

        let payload = match resp {
            Message::Response {
                request_id,
                payload,
                ..
            } => {
                if request_id != 1 {
                    return Err(format!("response request_id mismatch: {request_id}"));
                }
                payload
            }
            Message::Goodbye { reason } => return Err(format!("unexpected Goodbye: {reason}")),
            other => return Err(format!("expected Response, got {other:?}")),
        };

        let decoded: Result<i64, RoamError<Never>> =
            facet_postcard::from_slice(&payload).map_err(|e| format!("postcard resp: {e}"))?;

        match decoded {
            Ok(sum) => {
                if sum != 15 {
                    // 1 + 2 + 3 + 4 + 5 = 15
                    return Err(format!("expected sum 15, got {sum}"));
                }
            }
            Err(e) => return Err(format!("expected Ok response, got Err({e:?})")),
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify streaming.server-to-client] - Server pushes data to client
// r[verify streaming.data] - Data messages carry stream payloads
// r[verify streaming.close] - Close terminates stream gracefully
#[test]
fn streaming_generate_server_to_client() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;
        hello_exchange(&mut io).await?;

        // Get the method ID for `generate(count: u32, output: Tx<i32>)`
        // Tx<i32> serializes as u64 on the wire (stream ID)
        let method_id = testbed_method_id(
            "generate",
            vec![
                ArgDetail {
                    name: "count".into(),
                    ty: <u32 as Facet>::SHAPE,
                },
                ArgDetail {
                    name: "output".into(),
                    ty: <u64 as Facet>::SHAPE, // Tx<i32> is proxy'd to u64
                },
            ],
            <() as Facet>::SHAPE,
        );

        // Allocate stream ID (odd = initiator)
        let stream_id: u64 = 1;
        let count: u32 = 5;

        // Send Request with (count, stream_id)
        let req_payload = facet_postcard::to_vec(&(count, stream_id))
            .map_err(|e| format!("postcard args: {e}"))?;
        let req = Message::Request {
            request_id: 1,
            method_id,
            metadata: metadata_empty(),
            payload: req_payload,
        };
        io.send(&req).await.map_err(|e| e.to_string())?;

        // Collect Data messages from server
        let mut received: Vec<i32> = Vec::new();
        let mut got_close = false;
        let mut got_response = false;

        // Keep receiving until we have both Close and Response
        while !got_close || !got_response {
            let msg = io
                .recv_timeout(Duration::from_millis(500))
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!(
                    "connection closed unexpectedly (got_close={got_close}, got_response={got_response}, received={received:?})"
                ))?;

            match msg {
                Message::Data { stream_id: sid, payload } => {
                    if sid != stream_id {
                        return Err(format!("unexpected stream_id {sid}, expected {stream_id}"));
                    }
                    let n: i32 = facet_postcard::from_slice(&payload)
                        .map_err(|e| format!("postcard data: {e}"))?;
                    received.push(n);
                }
                Message::Close { stream_id: sid } => {
                    if sid != stream_id {
                        return Err(format!("close stream_id mismatch: {sid}"));
                    }
                    got_close = true;
                }
                Message::Response { request_id, .. } => {
                    if request_id != 1 {
                        return Err(format!("response request_id mismatch: {request_id}"));
                    }
                    got_response = true;
                }
                Message::Goodbye { reason } => {
                    return Err(format!("unexpected Goodbye: {reason}"));
                }
                other => {
                    return Err(format!("unexpected message: {other:?}"));
                }
            }
        }

        // Verify received numbers
        let expected: Vec<i32> = (0..count as i32).collect();
        if received != expected {
            return Err(format!("expected {expected:?}, got {received:?}"));
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify streaming.bidirectional] - Both sides can push data
// r[verify streaming.lifecycle.concurrent] - Input/output streams are independent
#[test]
fn streaming_transform_bidirectional() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;
        hello_exchange(&mut io).await?;

        // Get the method ID for `transform(input: Rx<String>, output: Tx<String>)`
        let method_id = testbed_method_id(
            "transform",
            vec![
                ArgDetail {
                    name: "input".into(),
                    ty: <u64 as Facet>::SHAPE, // Rx<String> is proxy'd to u64
                },
                ArgDetail {
                    name: "output".into(),
                    ty: <u64 as Facet>::SHAPE, // Tx<String> is proxy'd to u64
                },
            ],
            <() as Facet>::SHAPE,
        );

        // Allocate stream IDs (odd = initiator)
        let input_stream_id: u64 = 1;
        let output_stream_id: u64 = 3;

        // Send Request with (input_stream_id, output_stream_id)
        let req_payload = facet_postcard::to_vec(&(input_stream_id, output_stream_id))
            .map_err(|e| format!("postcard args: {e}"))?;
        let req = Message::Request {
            request_id: 1,
            method_id,
            metadata: metadata_empty(),
            payload: req_payload,
        };
        io.send(&req).await.map_err(|e| e.to_string())?;

        // Send some strings and collect echoes
        let messages = ["hello", "world", "test"];
        let mut received: Vec<String> = Vec::new();

        for msg in &messages {
            // Send input
            let data_payload = facet_postcard::to_vec(&msg.to_string())
                .map_err(|e| format!("postcard data: {e}"))?;
            io.send(&Message::Data {
                stream_id: input_stream_id,
                payload: data_payload,
            })
            .await
            .map_err(|e| e.to_string())?;

            // Receive echo on output stream
            let resp_msg = io
                .recv_timeout(Duration::from_millis(500))
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "expected Data from subject".to_string())?;

            match resp_msg {
                Message::Data { stream_id, payload } => {
                    if stream_id != output_stream_id {
                        return Err(format!(
                            "unexpected stream_id {stream_id}, expected {output_stream_id}"
                        ));
                    }
                    let s: String = facet_postcard::from_slice(&payload)
                        .map_err(|e| format!("postcard data: {e}"))?;
                    received.push(s);
                }
                other => return Err(format!("expected Data, got {other:?}")),
            }
        }

        // Close input stream
        io.send(&Message::Close {
            stream_id: input_stream_id,
        })
        .await
        .map_err(|e| e.to_string())?;

        // Expect Close on output stream and Response (order may vary)
        let mut got_close = false;
        let mut got_response = false;

        while !got_close || !got_response {
            let msg = io
                .recv_timeout(Duration::from_millis(500))
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "expected Close/Response from subject".to_string())?;

            match msg {
                Message::Close { stream_id } => {
                    if stream_id != output_stream_id {
                        return Err(format!(
                            "close stream_id mismatch: {stream_id}, expected {output_stream_id}"
                        ));
                    }
                    got_close = true;
                }
                Message::Response { request_id, .. } => {
                    if request_id != 1 {
                        return Err(format!("response request_id mismatch: {request_id}"));
                    }
                    got_response = true;
                }
                other => return Err(format!("expected Close or Response, got {other:?}")),
            }
        }

        // Verify echoes
        let expected: Vec<String> = messages.iter().map(|s| s.to_string()).collect();
        if received != expected {
            return Err(format!("expected {expected:?}, got {received:?}"));
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}
