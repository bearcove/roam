use std::convert::Infallible;
use std::time::Duration;

use roam_types::{Hello, Message, MetadataValue, Payload, RequestId, RoamError};
use spec_proto::MathError;
use spec_tests::harness::{accept_subject, our_hello, run_async};
use spec_tests::testbed::method_id;

fn metadata_empty() -> Vec<(String, MetadataValue, u64)> {
    Vec::new()
}

// r[verify call.initiate] - Call initiated by sending Request message
// r[verify call.complete] - Response has matching request_id
// r[verify call.lifecycle.single-response] - Exactly one response per request
// r[verify call.lifecycle.ordering] - Response correlated by request_id
// r[verify call.request-id.uniqueness] - Uses unique request_id (1)
// r[verify call.metadata.type] - Metadata is Vec<(String, MetadataValue)>
// r[verify call.request.payload-encoding] - Payload is POSTCARD tuple of args
// r[verify call.response.encoding] - Response is POSTCARD Result<T, RoamError<E>>
// r[verify transport.message.binary] - Binary transport (TCP stream)
#[test]
fn rpc_echo_roundtrip() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;

        // Subject hello first.
        let msg = io
            .recv_timeout(Duration::from_millis(250))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected Hello from subject".to_string())?;
        match msg {
            Message::Hello(Hello::V6 { .. }) => {}
            _ => return Err(format!("expected Hello::V6, got {msg:?}")),
        }

        io.send(&Message::Hello(our_hello(1024 * 1024)))
            .await
            .map_err(|e| e.to_string())?;

        let req_payload = facet_postcard::to_vec(&(String::from("hello"),))
            .map_err(|e| format!("postcard args: {e}"))?;
        let req = Message::Request {
            conn_id: roam_types::ConnectionId::ROOT,
            request_id: RequestId(1),
            method_id: method_id::echo(),
            metadata: metadata_empty(),
            channels: vec![],
            payload: Payload(req_payload),
        };
        io.send(&req).await.map_err(|e| e.to_string())?;

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
                if request_id != RequestId(1) {
                    return Err(format!("response request_id mismatch: {request_id}"));
                }
                payload
            }
            Message::Goodbye { reason, .. } => return Err(format!("unexpected Goodbye: {reason}")),
            other => return Err(format!("expected Response, got {other:?}")),
        };

        let decoded: Result<String, RoamError<Infallible>> =
            facet_postcard::from_slice(&payload.0).map_err(|e| format!("postcard resp: {e}"))?;

        match decoded {
            Ok(s) => {
                if s != "hello" {
                    return Err(format!("expected echo payload \"hello\", got {s:?}"));
                }
            }
            Err(e) => return Err(format!("expected Ok response, got Err({e:?})")),
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify call.error.user] - User error from fallible method is returned as RoamError::User(E)
// r[verify call.response.encoding] - Response is POSTCARD Result<T, RoamError<E>>
#[test]
fn rpc_user_error_roundtrip() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;

        // Hello exchange.
        let _ = io
            .recv_timeout(Duration::from_millis(250))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected Hello from subject".to_string())?;
        io.send(&Message::Hello(our_hello(1024 * 1024)))
            .await
            .map_err(|e| e.to_string())?;

        // Call divide(10, 0) - should return Err(MathError::DivisionByZero)
        let req_payload =
            facet_postcard::to_vec(&(10i64, 0i64)).map_err(|e| format!("postcard args: {e}"))?;
        let req = Message::Request {
            conn_id: roam_types::ConnectionId::ROOT,
            request_id: RequestId(100),
            method_id: method_id::divide(),
            metadata: metadata_empty(),
            channels: vec![],
            payload: Payload(req_payload),
        };
        io.send(&req).await.map_err(|e| e.to_string())?;

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
                if request_id != RequestId(100) {
                    return Err(format!("response request_id mismatch: {request_id}"));
                }
                payload
            }
            Message::Goodbye { reason, .. } => return Err(format!("unexpected Goodbye: {reason}")),
            other => return Err(format!("expected Response, got {other:?}")),
        };

        // The response should be Result<i64, RoamError<MathError>> = Err(User(DivisionByZero))
        let decoded: Result<i64, RoamError<MathError>> =
            facet_postcard::from_slice(&payload.0).map_err(|e| format!("postcard resp: {e}"))?;

        match decoded {
            Ok(v) => {
                return Err(format!("expected Err(User(DivisionByZero)), got Ok({v})"));
            }
            Err(RoamError::User(MathError::DivisionByZero)) => {
                // Success! The user error was properly roundtripped.
            }
            Err(other) => {
                return Err(format!(
                    "expected Err(User(DivisionByZero)), got Err({other:?})"
                ));
            }
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify call.error.unknown-method] - Unknown method_id returns UnknownMethod error
// r[verify call.error.roam-error] - Protocol errors use RoamError variants
// r[verify call.error.protocol] - UnknownMethod is a protocol-level error (discriminant 1)
#[test]
fn rpc_unknown_method_returns_unknownmethod_error() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;

        // Hello exchange.
        let _ = io
            .recv_timeout(Duration::from_millis(250))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected Hello from subject".to_string())?;
        io.send(&Message::Hello(our_hello(1024 * 1024)))
            .await
            .map_err(|e| e.to_string())?;

        // Well-formed request with an unknown method id.
        let req_payload = facet_postcard::to_vec(&(String::from("hello"),))
            .map_err(|e| format!("postcard args: {e}"))?;
        let req = Message::Request {
            conn_id: roam_types::ConnectionId::ROOT,
            request_id: RequestId(2),
            method_id: MethodId(0xdeadbeef),
            metadata: metadata_empty(),
            channels: vec![],
            payload: Payload(req_payload),
        };
        io.send(&req).await.map_err(|e| e.to_string())?;

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
                if request_id != RequestId(2) {
                    return Err(format!("response request_id mismatch: {request_id}"));
                }
                payload
            }
            Message::Goodbye { reason, .. } => return Err(format!("unexpected Goodbye: {reason}")),
            other => return Err(format!("expected Response, got {other:?}")),
        };

        let decoded: Result<String, RoamError<Infallible>> =
            facet_postcard::from_slice(&payload.0).map_err(|e| format!("postcard resp: {e}"))?;

        match decoded {
            Ok(v) => return Err(format!("expected Err(UnknownMethod), got Ok({v:?})")),
            Err(RoamError::UnknownMethod) => {}
            Err(other) => return Err(format!("expected UnknownMethod, got {other:?}")),
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify call.error.invalid-payload] - Malformed payload returns InvalidPayload error
#[test]
fn rpc_invalid_payload_returns_invalidpayload_error() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;

        // Hello exchange.
        let _ = io
            .recv_timeout(Duration::from_millis(250))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected Hello from subject".to_string())?;
        io.send(&Message::Hello(our_hello(1024 * 1024)))
            .await
            .map_err(|e| e.to_string())?;

        // Send request with invalid payload (random bytes, not valid postcard).
        let req = Message::Request {
            conn_id: roam_types::ConnectionId::ROOT,
            request_id: RequestId(3),
            method_id: method_id::echo(),
            metadata: metadata_empty(),
            channels: vec![],
            payload: Payload(vec![0xff, 0xff, 0xff, 0xff]), // Invalid postcard data
        };
        io.send(&req).await.map_err(|e| e.to_string())?;

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
                if request_id != RequestId(3) {
                    return Err(format!("response request_id mismatch: {request_id}"));
                }
                payload
            }
            Message::Goodbye { reason, .. } => return Err(format!("unexpected Goodbye: {reason}")),
            other => return Err(format!("expected Response, got {other:?}")),
        };

        let decoded: Result<String, RoamError<Infallible>> =
            facet_postcard::from_slice(&payload.0).map_err(|e| format!("postcard resp: {e}"))?;

        match decoded {
            Ok(v) => return Err(format!("expected Err(InvalidPayload), got Ok({v:?})")),
            Err(RoamError::InvalidPayload) => {}
            Err(other) => return Err(format!("expected InvalidPayload, got {other:?}")),
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify call.pipelining.allowed] - Multiple requests in flight simultaneously
// r[verify call.pipelining.independence] - Each request is independent
// r[verify core.call] - Each call has one Request and one Response
// r[verify core.call.request-id] - Request IDs correlate requests to responses
#[test]
fn rpc_pipelining_multiple_requests() {
    run_async(async {
        let (mut io, mut child) = accept_subject().await?;

        // Hello exchange.
        let _ = io
            .recv_timeout(Duration::from_millis(250))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected Hello from subject".to_string())?;
        io.send(&Message::Hello(our_hello(1024 * 1024)))
            .await
            .map_err(|e| e.to_string())?;

        // Send 3 requests without waiting for responses (pipelining).
        let messages = ["first", "second", "third"];
        for (i, msg) in messages.iter().enumerate() {
            let req_payload = facet_postcard::to_vec(&(msg.to_string(),))
                .map_err(|e| format!("postcard args: {e}"))?;
            let req = Message::Request {
                conn_id: roam_types::ConnectionId::ROOT,
                request_id: RequestId((i + 10) as u32), // Use 10, 11, 12 to distinguish from other tests
                method_id: method_id::echo(),
                metadata: metadata_empty(),
                channels: vec![],
                payload: Payload(req_payload),
            };
            io.send(&req).await.map_err(|e| e.to_string())?;
        }

        // Collect all 3 responses (may arrive in any order).
        let mut responses: std::collections::HashMap<RequestId, String> =
            std::collections::HashMap::new();
        for _ in 0..3 {
            let resp = io
                .recv_timeout(Duration::from_millis(500))
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "expected Response from subject".to_string())?;

            match resp {
                Message::Response {
                    request_id,
                    payload,
                    ..
                } => {
                    let decoded: Result<String, RoamError<Infallible>> =
                        facet_postcard::from_slice(&payload.0)
                            .map_err(|e| format!("postcard resp: {e}"))?;
                    match decoded {
                        Ok(s) => {
                            responses.insert(request_id, s);
                        }
                        Err(e) => return Err(format!("expected Ok, got Err({e:?})")),
                    }
                }
                other => return Err(format!("expected Response, got {other:?}")),
            }
        }

        // Verify all 3 responses received with correct correlation.
        for (i, msg) in messages.iter().enumerate() {
            let request_id = RequestId((i + 10) as u32);
            match responses.get(&request_id) {
                Some(s) if s == *msg => {}
                Some(s) => {
                    return Err(format!(
                        "request_id {request_id}: expected {msg:?}, got {s:?}"
                    ));
                }
                None => return Err(format!("missing response for request_id {request_id}")),
            }
        }

        let _ = child.kill().await;
        Ok::<_, String>(())
    })
    .unwrap();
}
