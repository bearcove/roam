use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use moire::task::FutureExt;
use roam_types::{
    Caller, Handler, MessageFamily, Metadata, MethodId, Parity, Payload, ReplySink, RequestBody,
    RequestCall, RequestCancel, RequestMessage, RequestResponse, RoamError, SelfRef,
};

use crate::session::{ConnectionMessage, acceptor, initiator};
use crate::{BareConduit, Driver, DriverReplySink, memory_link_pair};

type MessageConduit = BareConduit<MessageFamily, crate::MemoryLink>;

fn message_conduit_pair() -> (MessageConduit, MessageConduit) {
    let (a, b) = memory_link_pair(64);
    (BareConduit::new(a), BareConduit::new(b))
}

/// A handler that echoes back the raw args payload as the response.
struct EchoHandler;

impl Handler<DriverReplySink> for EchoHandler {
    fn handle(
        &self,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        async move {
            let args_bytes = match &call.args {
                Payload::Incoming(bytes) => *bytes,
                _ => panic!("expected incoming payload"),
            };

            let result: u32 = facet_postcard::from_slice(args_bytes).expect("deserialize args");
            reply
                .send_reply(RequestResponse {
                    ret: Payload::outgoing(&result),
                    channels: vec![],
                    metadata: Default::default(),
                })
                .await;
        }
    }
}

/// A no-op handler — the client side doesn't expect incoming calls in this test.
struct NoopHandler;

impl Handler<DriverReplySink> for NoopHandler {
    fn handle(
        &self,
        _call: SelfRef<RequestCall<'static>>,
        _reply: DriverReplySink,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        async {}
    }
}

/// A handler that blocks forever until its task is cancelled.
/// Tracks whether cancellation occurred via a drop guard.
struct BlockingHandler {
    was_cancelled: Arc<AtomicBool>,
}

impl Handler<DriverReplySink> for BlockingHandler {
    fn handle(
        &self,
        _call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        let was_cancelled = self.was_cancelled.clone();
        async move {
            // Hold the reply to prevent premature DriverReplySink::drop
            let _reply = reply;
            // Create a drop guard that records cancellation
            struct DropGuard(Arc<AtomicBool>);
            impl Drop for DropGuard {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::SeqCst);
                }
            }
            let _guard = DropGuard(was_cancelled);
            // Block forever — only cancellation (abort) will stop this
            std::future::pending::<()>().await;
        }
    }
}

// r[verify rpc.cancel]
// r[verify rpc.cancel.channels]
#[tokio::test]
async fn cancel_aborts_in_flight_handler() {
    let (client_conduit, server_conduit) = message_conduit_pair();

    let was_cancelled = Arc::new(AtomicBool::new(false));
    let was_cancelled_check = was_cancelled.clone();

    let server_task = moire::task::spawn(
        async move {
            let (mut server_session, server_handle) = acceptor(server_conduit)
                .establish()
                .await
                .expect("server handshake failed");
            let mut server_driver = Driver::new(
                server_handle,
                BlockingHandler { was_cancelled },
                Parity::Even,
            );
            moire::task::spawn(async move { server_session.run().await }.named("server_session"));
            moire::task::spawn(async move { server_driver.run().await }.named("server_driver"));
        }
        .named("server_setup"),
    );

    // Set up client side. We need both the Caller (for sending the call) and
    // the raw sender (for sending the cancel message with the same request ID).
    let (mut client_session, client_handle) = initiator(client_conduit)
        .establish()
        .await
        .expect("client handshake failed");
    let client_sender = client_handle.sender.clone();
    let mut client_driver = Driver::new(client_handle, NoopHandler, Parity::Odd);
    let caller = client_driver.caller();
    moire::task::spawn(async move { client_session.run().await }.named("client_session"));
    moire::task::spawn(async move { client_driver.run().await }.named("client_driver"));

    server_task.await.expect("server setup failed");

    // Spawn the call as a task so we can concurrently send a cancel.
    let call_task = moire::task::spawn(
        async move {
            let args_value: u32 = 99;
            caller
                .call(RequestCall {
                    method_id: MethodId(1),
                    args: Payload::outgoing(&args_value),
                    channels: vec![],
                    metadata: Default::default(),
                })
                .await
        }
        .named("client_call"),
    );

    // Give the call time to reach the server and start the handler.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Send a cancel for request ID 1 (the first request on an Odd-parity
    // connection allocates ID 1).
    let cancel_req_id = roam_types::RequestId(1);
    client_sender
        .send(ConnectionMessage::Request(RequestMessage {
            id: cancel_req_id,
            body: RequestBody::Cancel(RequestCancel {
                metadata: Metadata::default(),
            }),
        }))
        .await
        .expect("send cancel");

    // The call should resolve with a response containing RoamError::Cancelled.
    let result = call_task.await.expect("call task join");
    let response = result.expect("call should receive a response");
    let ret_bytes = match &response.ret {
        Payload::Incoming(bytes) => *bytes,
        _ => panic!("expected incoming payload in response"),
    };
    let error: RoamError = facet_postcard::from_slice(ret_bytes).expect("deserialize response");
    assert!(
        matches!(error, RoamError::Cancelled),
        "expected RoamError::Cancelled in response payload"
    );

    // Wait for the handler abort to propagate (drop guard sets the flag).
    for _ in 0..20 {
        if was_cancelled_check.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // Verify the handler was actually cancelled (drop guard fired).
    assert!(
        was_cancelled_check.load(Ordering::SeqCst),
        "handler should have been cancelled"
    );
}

#[tokio::test]
async fn echo_call_across_memory_link() {
    let (client_conduit, server_conduit) = message_conduit_pair();

    // Server and client handshakes must run concurrently — both sides exchange
    // settings before either can proceed.
    let server_task = moire::task::spawn(
        async move {
            let (mut server_session, server_handle) = acceptor(server_conduit)
                .establish()
                .await
                .expect("server handshake failed");
            let mut server_driver = Driver::new(server_handle, EchoHandler, Parity::Even);
            moire::task::spawn(async move { server_session.run().await }.named("server_session"));
            moire::task::spawn(async move { server_driver.run().await }.named("server_driver"));
        }
        .named("server_setup"),
    );

    // Set up client side (runs concurrently with server_task above).
    let (mut client_session, client_handle) = initiator(client_conduit)
        .establish()
        .await
        .expect("client handshake failed");
    let mut client_driver = Driver::new(client_handle, NoopHandler, Parity::Odd);
    let caller = client_driver.caller();
    moire::task::spawn(async move { client_session.run().await }.named("client_session"));
    moire::task::spawn(async move { client_driver.run().await }.named("client_driver"));

    server_task.await.expect("server setup failed");

    // Make a call: serialize a u32 as the args payload.
    let args_value: u32 = 42;
    let response = caller
        .call(RequestCall {
            method_id: MethodId(1),
            args: Payload::outgoing(&args_value),
            channels: vec![],
            metadata: Default::default(),
        })
        .await
        .expect("call should succeed");

    // The echo handler sends back the same bytes. Deserialize the response.
    let ret_bytes = match &response.ret {
        Payload::Incoming(bytes) => *bytes,
        _ => panic!("expected incoming payload in response"),
    };
    let result: u32 = facet_postcard::from_slice(ret_bytes).expect("deserialize response");
    assert_eq!(result, 42);
}
