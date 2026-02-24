use roam_types::{
    Caller, ConnectionSettings, Handler, MessageFamily, MethodId, Parity, Payload, ReplySink,
    RequestCall, RequestResponse, SelfRef,
};

use crate::session::session_no_handshake;
use crate::{BareConduit, Driver, DriverReplySink, memory_link_pair};

type MessageConduit = BareConduit<MessageFamily, crate::MemoryLink>;

fn message_conduit_pair() -> (MessageConduit, MessageConduit) {
    let (a, b) = memory_link_pair(64);
    (BareConduit::new(a), BareConduit::new(b))
}

fn default_settings(parity: Parity) -> ConnectionSettings {
    ConnectionSettings {
        parity,
        max_concurrent_requests: 64,
    }
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
            // The incoming args are Payload::Incoming(bytes).
            // Echo them back as-is in the response.
            let args_bytes = match &call.args {
                Payload::Incoming(bytes) => *bytes,
                _ => panic!("expected incoming payload"),
            };

            // Build response with the same bytes.
            reply
                .send_reply(RequestResponse {
                    ret: Payload::Incoming(args_bytes),
                    channels: &[],
                    metadata: Default::default(),
                })
                .await
                .ok();
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

    // Set up server side.
    let (server_handle, server_recv_loop) = session_no_handshake(
        server_conduit,
        default_settings(Parity::Even),
        default_settings(Parity::Odd),
    );
    let mut server_driver = Driver::new(server_handle, EchoHandler, Parity::Even);
    moire::task::spawn(server_recv_loop).named("server_recv_loop");
    moire::task::spawn(async move { server_driver.run().await }).named("server_driver");

    // Set up client side.
    let (client_handle, client_recv_loop) = session_no_handshake(
        client_conduit,
        default_settings(Parity::Odd),
        default_settings(Parity::Even),
    );
    let mut client_driver = Driver::new(client_handle, NoopHandler, Parity::Odd);
    let caller = client_driver.caller();
    moire::task::spawn(client_recv_loop).named("client_recv_loop");
    moire::task::spawn(async move { client_driver.run().await }).named("client_driver");

    // Make a call: serialize a u32 as the args payload.
    let args_value: u32 = 42;
    let response = caller
        .call(RequestCall {
            method_id: MethodId(1),
            args: Payload::outgoing(&args_value),
            channels: &[],
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
