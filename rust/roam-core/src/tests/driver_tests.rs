use moire::task::FutureExt;
use roam_types::{
    Caller, ConnectionSettings, Handler, MessageFamily, MethodId, Parity, Payload, ReplySink,
    RequestCall, RequestResponse, SelfRef,
};

use crate::session::{acceptor, initiator};
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
