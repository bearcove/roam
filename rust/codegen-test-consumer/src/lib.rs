//! Test consumer crate that uses generated code from build.rs.

// Include the generated code
include!(concat!(env!("OUT_DIR"), "/generated.rs"));

#[cfg(test)]
mod tests {
    use super::calculator::*;

    #[test]
    fn method_ids_generated() {
        // Verify method ID constants are generated
        assert_ne!(method_id::ADD, 0);
        assert_ne!(method_id::MULTIPLY, 0);
        // They should be different
        assert_ne!(method_id::ADD, method_id::MULTIPLY);
    }

    // Test that we can implement the handler trait
    #[derive(Clone)]
    struct TestCalculator;

    impl CalculatorHandler for TestCalculator {
        async fn add(&self, a: i32, b: i32) -> Result<i32, RoamError<Never>> {
            Ok(a + b)
        }

        async fn multiply(&self, a: i32, b: i32) -> Result<i32, RoamError<Never>> {
            Ok(a * b)
        }

        async fn sum_stream(&self, mut numbers: Rx<i32>) -> Result<i64, RoamError<Never>> {
            let mut sum: i64 = 0;
            while let Some(n) = numbers.recv().await.ok().flatten() {
                sum += n as i64;
            }
            Ok(sum)
        }

        async fn range(&self, count: u32, output: Tx<u32>) -> Result<(), RoamError<Never>> {
            for i in 0..count {
                let _ = output.send(&i).await;
            }
            Ok(())
        }
    }

    #[test]
    fn can_create_dispatcher() {
        let _dispatcher = CalculatorDispatcher::new(TestCalculator);
        // Dispatcher implements ServiceDispatcher trait
    }

    /// Test that Rust-generated client can talk to Rust-generated server over TCP.
    #[tokio::test]
    async fn rust_to_rust_tcp_roundtrip() {
        use roam::__private::facet_postcard;
        use roam_stream::{Message, Server};
        use std::time::Duration;
        use tokio::net::TcpListener;

        // 1. Bind listener on random port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // 2. Spawn server task with CalculatorDispatcher
        let server_handle = tokio::spawn(async move {
            let server = Server::new();
            let mut conn = server.accept(&listener).await.unwrap();
            let dispatcher = CalculatorDispatcher::new(TestCalculator);
            conn.run(&dispatcher).await
        });

        // 3. Connect as client
        let client = Server::new();
        let mut conn = client.connect(&addr.to_string()).await.unwrap();

        // 4. Test add(2, 3) = 5
        let payload = facet_postcard::to_vec(&(2i32, 3i32)).unwrap();
        conn.io()
            .send(&Message::Request {
                request_id: 1,
                method_id: method_id::ADD,
                metadata: vec![],
                payload,
            })
            .await
            .unwrap();

        let resp = conn
            .io()
            .recv_timeout(Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();
        let Message::Response {
            request_id,
            payload,
            ..
        } = resp
        else {
            panic!("expected Response, got {resp:?}")
        };
        assert_eq!(request_id, 1);
        // Response is CallResult<T, Never> = Result<T, RoamError<Never>> per spec r[unary.response.encoding]
        let result: CallResult<i32, Never> = facet_postcard::from_slice(&payload).unwrap();
        assert_eq!(result, Ok(5));

        // 5. Test multiply(4, 7) = 28
        let payload = facet_postcard::to_vec(&(4i32, 7i32)).unwrap();
        conn.io()
            .send(&Message::Request {
                request_id: 2,
                method_id: method_id::MULTIPLY,
                metadata: vec![],
                payload,
            })
            .await
            .unwrap();

        let resp = conn
            .io()
            .recv_timeout(Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();
        let Message::Response {
            request_id,
            payload,
            ..
        } = resp
        else {
            panic!("expected Response, got {resp:?}")
        };
        assert_eq!(request_id, 2);
        let result: CallResult<i32, Never> = facet_postcard::from_slice(&payload).unwrap();
        assert_eq!(result, Ok(28));

        // 6. Drop connection - server will see clean shutdown
        drop(conn);
        let _ = server_handle.await;
    }

    // NOTE: duplicate-detection requires concurrent request handling to test.
    // With synchronous processing, requests complete before the next is received.
    // See r[unary.request-id.duplicate-detection] in the spec.

    /// r[verify streaming.id.zero-reserved] - Stream ID 0 is reserved.
    #[tokio::test]
    async fn stream_id_zero_rejected() {
        use roam_stream::{Message, Server};
        use std::time::Duration;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_handle = tokio::spawn(async move {
            let server = Server::new();
            let mut conn = server.accept(&listener).await.unwrap();
            let dispatcher = CalculatorDispatcher::new(TestCalculator);
            conn.run(&dispatcher).await
        });

        let client = Server::new();
        let mut conn = client.connect(&addr.to_string()).await.unwrap();

        // Send Data with stream_id = 0 (reserved, protocol violation)
        conn.io()
            .send(&Message::Data {
                stream_id: 0,
                payload: vec![1, 2, 3],
            })
            .await
            .unwrap();

        // Server should send Goodbye with zero-reserved reason
        let msg = conn
            .io()
            .recv_timeout(Duration::from_secs(1))
            .await
            .unwrap();
        match msg {
            Some(Message::Goodbye { reason }) => {
                assert!(
                    reason.contains("zero-reserved"),
                    "expected zero-reserved, got: {reason}"
                );
            }
            other => panic!("expected Goodbye, got {other:?}"),
        }

        let _ = server_handle.await;
    }

    /// r[verify message.hello.enforcement] - Exceeding negotiated payload limit triggers Goodbye.
    /// r[verify streaming.data.size-limit] - Stream data bounded by max_payload_size.
    #[tokio::test]
    async fn oversized_stream_data_rejected() {
        use roam_stream::{Message, Server};
        use std::time::Duration;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_handle = tokio::spawn(async move {
            let server = Server::new();
            let mut conn = server.accept(&listener).await.unwrap();
            let dispatcher = CalculatorDispatcher::new(TestCalculator);
            conn.run(&dispatcher).await
        });

        let client = Server::new();
        let mut conn = client.connect(&addr.to_string()).await.unwrap();

        // Default max_payload_size is 1MB. Send Data larger than that.
        // Use stream_id = 1 (valid odd ID for initiator streams).
        // Use 1MB + 1 byte to be just over the limit
        let oversized_payload = vec![0u8; 1024 * 1024 + 1];
        conn.io()
            .send(&Message::Data {
                stream_id: 1,
                payload: oversized_payload,
            })
            .await
            .unwrap();

        // Server should send Goodbye with flow.unary.payload-limit reason (payload exceeded)
        // r[impl flow.unary.payload-limit] identifies the rule being violated
        let msg = conn
            .io()
            .recv_timeout(Duration::from_secs(1))
            .await
            .unwrap();
        match msg {
            Some(Message::Goodbye { reason }) => {
                assert!(
                    reason.contains("flow.unary.payload-limit"),
                    "expected flow.unary.payload-limit, got: {reason}"
                );
            }
            other => panic!("expected Goodbye, got {other:?}"),
        }

        let _ = server_handle.await;
    }

    /// Test streaming RPC: sum_stream with Tx/Rx streams.
    ///
    /// This test verifies the full streaming flow:
    /// 1. Client creates Tx<i32> stream via ConnectionHandle
    /// 2. Sends request with stream ID
    /// 3. Sends Data messages with numbers
    /// 4. Closes the stream
    /// 5. Server sums all received numbers
    /// 6. Server responds with the sum
    #[tokio::test]
    async fn streaming_sum_stream_roundtrip() {
        use roam::__private::facet_postcard;
        use roam_stream::{CobsFramed, Hello, establish_acceptor, establish_initiator};
        use std::time::Duration;
        use tokio::net::TcpListener;

        let hello = Hello::V1 {
            max_payload_size: 1024 * 1024,
            initial_stream_credit: 64 * 1024,
        };

        // 1. Bind listener on random port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // 2. Spawn server task with CalculatorDispatcher
        let server_hello = hello.clone();
        let server_handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = CobsFramed::new(stream);
            let dispatcher = CalculatorDispatcher::new(TestCalculator);
            let (_handle, driver) = establish_acceptor(io, server_hello, dispatcher)
                .await
                .unwrap();
            // Run the driver (this processes requests until connection closes)
            let _ = driver.run().await;
        });

        // 3. Connect as client
        let stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
        let io = CobsFramed::new(stream);

        // Create a simple no-op dispatcher for the client side
        struct NoOpDispatcher;
        impl roam::session::ServiceDispatcher for NoOpDispatcher {
            fn is_streaming(&self, _method_id: u64) -> bool {
                false
            }
            async fn dispatch_unary(
                &self,
                _method_id: u64,
                _payload: &[u8],
            ) -> Result<Vec<u8>, String> {
                Ok(vec![1, 1])
            }
            fn dispatch_streaming(
                &self,
                _method_id: u64,
                _payload: Vec<u8>,
                _registry: &mut roam::session::StreamRegistry,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + Send + 'static>,
            > {
                Box::pin(async { Ok(vec![1, 1]) })
            }
        }

        let (handle, driver) = establish_initiator(io, hello.clone(), NoOpDispatcher)
            .await
            .unwrap();

        // Spawn the driver
        let driver_handle = tokio::spawn(async move {
            let _ = driver.run().await;
        });

        // 4. Create a Tx<i32> stream for sending numbers
        let tx: Tx<i32> = handle.new_tx();
        let stream_id = tx.stream_id();

        // 5. Build the request payload: tuple of (stream_id as u64)
        // The wire format encodes Tx<T> as just the u64 stream_id via proxy
        let payload = facet_postcard::to_vec(&(stream_id,)).unwrap();

        // 6. Send the streaming request using call_raw (low level)
        // We need to spawn this as a separate task because we need to send data
        // before the response arrives
        let call_handle = {
            let handle = handle.clone();
            tokio::spawn(async move { handle.call_raw(method_id::SUM_STREAM, payload).await })
        };

        // 7. Send numbers via the Tx stream
        // Small delay to let the request be processed
        tokio::time::sleep(Duration::from_millis(50)).await;

        tx.send(&1i32).await.unwrap();
        tx.send(&2i32).await.unwrap();
        tx.send(&3i32).await.unwrap();
        tx.send(&4i32).await.unwrap();

        // 8. Close the stream by dropping Tx
        drop(tx);

        // Small delay for close to propagate
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 9. Wait for response
        let response = tokio::time::timeout(Duration::from_secs(2), call_handle)
            .await
            .expect("timeout waiting for response")
            .expect("join error")
            .expect("call error");

        // 10. Decode response: CallResult<i64, Never> = Result<i64, RoamError<Never>>
        // Wire format: [0] for Ok followed by varint-encoded i64
        let result: CallResult<i64, Never> = facet_postcard::from_slice(&response).unwrap();
        assert_eq!(result, Ok(10i64)); // 1 + 2 + 3 + 4 = 10

        // Cleanup
        driver_handle.abort();
        server_handle.abort();
    }

    /// Test streaming RPC: range with server-to-client streaming.
    ///
    /// This test verifies the reverse direction:
    /// 1. Client creates Rx<u32> stream (to receive from server)
    /// 2. Sends request with stream ID
    /// 3. Server handler receives Tx<u32> (due to inversion) and sends values
    /// 4. Client receives values via Rx stream
    /// 5. Server responds when done
    #[tokio::test]
    async fn streaming_range_roundtrip() {
        use roam::__private::facet_postcard;
        use roam_stream::{CobsFramed, Hello, establish_acceptor, establish_initiator};
        use std::time::Duration;
        use tokio::net::TcpListener;

        let hello = Hello::V1 {
            max_payload_size: 1024 * 1024,
            initial_stream_credit: 64 * 1024,
        };

        // 1. Bind listener on random port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // 2. Spawn server task with CalculatorDispatcher
        let server_hello = hello.clone();
        let server_handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = CobsFramed::new(stream);
            let dispatcher = CalculatorDispatcher::new(TestCalculator);
            let (_handle, driver) = establish_acceptor(io, server_hello, dispatcher)
                .await
                .unwrap();
            let _ = driver.run().await;
        });

        // 3. Connect as client
        let stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
        let io = CobsFramed::new(stream);

        // Create a simple no-op dispatcher for the client side
        struct NoOpDispatcher;
        impl roam::session::ServiceDispatcher for NoOpDispatcher {
            fn is_streaming(&self, _method_id: u64) -> bool {
                false
            }
            async fn dispatch_unary(
                &self,
                _method_id: u64,
                _payload: &[u8],
            ) -> Result<Vec<u8>, String> {
                Ok(vec![1, 1])
            }
            fn dispatch_streaming(
                &self,
                _method_id: u64,
                _payload: Vec<u8>,
                _registry: &mut roam::session::StreamRegistry,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + Send + 'static>,
            > {
                Box::pin(async { Ok(vec![1, 1]) })
            }
        }

        let (handle, driver) = establish_initiator(io, hello.clone(), NoOpDispatcher)
            .await
            .unwrap();

        // Spawn the driver
        let driver_handle = tokio::spawn(async move {
            let _ = driver.run().await;
        });

        // 4. Create an Rx<u32> stream for receiving numbers from server
        let rx: Rx<u32> = handle.new_rx();
        let stream_id = rx.stream_id();

        // 5. Build the request payload: tuple of (count: u32, output: stream_id as u64)
        // range(count: u32, output: Rx<u32>) - output is Rx from caller perspective
        let count: u32 = 5;
        let payload = facet_postcard::to_vec(&(count, stream_id)).unwrap();

        // 6. Send the streaming request
        let call_handle = {
            let handle = handle.clone();
            tokio::spawn(async move { handle.call_raw(method_id::RANGE, payload).await })
        };

        // 7. Receive numbers via the Rx stream
        let mut received = Vec::new();
        let mut rx = rx;
        while let Some(n) = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten()
        {
            received.push(n);
            if received.len() >= count as usize {
                break;
            }
        }

        // 8. Wait for response
        let response = tokio::time::timeout(Duration::from_secs(2), call_handle)
            .await
            .expect("timeout waiting for response")
            .expect("join error")
            .expect("call error");

        // 9. Verify results
        assert_eq!(received, vec![0, 1, 2, 3, 4], "should receive 0..5");

        // Response should be Ok(())
        let result: CallResult<(), Never> = facet_postcard::from_slice(&response).unwrap();
        assert_eq!(result, Ok(()));

        // Cleanup
        driver_handle.abort();
        server_handle.abort();
    }

    /// r[verify message.goodbye.receive] - Connection closes gracefully on Goodbye.
    #[tokio::test]
    async fn goodbye_closes_connection() {
        use roam_stream::{ConnectionError, Message, Server};
        use std::time::Duration;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_handle = tokio::spawn(async move {
            let server = Server::new();
            let mut conn = server.accept(&listener).await.unwrap();
            let dispatcher = CalculatorDispatcher::new(TestCalculator);
            conn.run(&dispatcher).await
        });

        let client = Server::new();
        let mut conn = client.connect(&addr.to_string()).await.unwrap();

        // Send Goodbye
        conn.io()
            .send(&Message::Goodbye {
                reason: "client-initiated-close".into(),
            })
            .await
            .unwrap();

        // Wait for server to process Goodbye and terminate
        let server_result = server_handle.await.unwrap();

        // Server should return Err(ConnectionError::Closed) when it receives Goodbye
        match server_result {
            Err(ConnectionError::Closed) => {
                // Expected: server received Goodbye and closed
            }
            Ok(()) => {
                // Also acceptable: connection closed cleanly (EOF before Goodbye processed)
            }
            Err(e) => panic!("unexpected error: {e:?}"),
        }

        // Connection should be closed - recv returns None
        let msg = conn
            .io()
            .recv_timeout(Duration::from_secs(1))
            .await
            .unwrap();
        assert!(
            msg.is_none(),
            "expected connection close (None), got {msg:?}"
        );
    }

    /// r[verify message.hello.enforcement] - Non-Hello before Hello is rejected.
    #[tokio::test]
    async fn non_hello_before_hello_rejected() {
        use roam_stream::{CobsFramed, Message};
        use std::time::Duration;
        use tokio::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Client sends Request before Hello - protocol violation!
        let client_handle = tokio::spawn(async move {
            let stream = TcpStream::connect(addr).await.unwrap();
            let mut io = CobsFramed::new(stream);

            // Send Request before Hello - protocol violation!
            io.send(&Message::Request {
                request_id: 1,
                method_id: 0,
                metadata: vec![],
                payload: vec![],
            })
            .await
            .unwrap();

            // Server sends Hello first (as part of accept), then sees our non-Hello and sends Goodbye
            let mut saw_hello = false;
            let mut saw_goodbye = false;
            let mut goodbye_reason = String::new();

            for _ in 0..3 {
                match io.recv_timeout(Duration::from_secs(1)).await.unwrap() {
                    Some(Message::Hello(_)) => saw_hello = true,
                    Some(Message::Goodbye { reason }) => {
                        saw_goodbye = true;
                        goodbye_reason = reason;
                        break;
                    }
                    None => break,
                    other => panic!("unexpected message: {other:?}"),
                }
            }

            (saw_hello, saw_goodbye, goodbye_reason)
        });

        // Server side: accept and run
        let server_handle = tokio::spawn(async move {
            use roam_stream::Server;
            let server = Server::new();
            server.accept(&listener).await
        });

        // Client should see Hello then Goodbye
        let (saw_hello, saw_goodbye, reason) = client_handle.await.unwrap();
        assert!(saw_hello, "expected to receive server's Hello");
        assert!(saw_goodbye, "expected to receive Goodbye");
        assert!(
            reason.contains("hello.ordering"),
            "expected hello.ordering, got: {reason}"
        );

        // Server should have returned error
        let server_result = server_handle.await.unwrap();
        assert!(server_result.is_err());
    }
}
