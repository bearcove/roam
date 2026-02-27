use moire::task::FutureExt;
use roam_shm::ShmLink;
use roam_shm::varslot::SizeClassConfig;
use roam_types::{
    Caller, Handler, MessageFamily, MethodId, Parity, Payload, ReplySink, RequestCall,
    RequestResponse, SelfRef,
};

use crate::session::{acceptor, initiator};
use crate::{BareConduit, Driver, DriverReplySink};

type MessageConduit = BareConduit<MessageFamily, ShmLink>;

fn message_conduit_pair() -> (MessageConduit, MessageConduit) {
    let classes = [SizeClassConfig {
        slot_size: 4096,
        slot_count: 16,
    }];
    let (a, b) = ShmLink::heap_pair(1 << 16, 1 << 20, 256, &classes);
    (BareConduit::new(a), BareConduit::new(b))
}

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
async fn echo_call_across_shm_link() {
    let (client_conduit, server_conduit) = message_conduit_pair();

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

    let (mut client_session, client_handle) = initiator(client_conduit)
        .establish()
        .await
        .expect("client handshake failed");
    let mut client_driver = Driver::new(client_handle, NoopHandler, Parity::Odd);
    let caller = client_driver.caller();
    moire::task::spawn(async move { client_session.run().await }.named("client_session"));
    moire::task::spawn(async move { client_driver.run().await }.named("client_driver"));

    server_task.await.expect("server setup failed");

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

    let ret_bytes = match &response.ret {
        Payload::Incoming(bytes) => *bytes,
        _ => panic!("expected incoming payload in response"),
    };
    let result: u32 = facet_postcard::from_slice(ret_bytes).expect("deserialize response");
    assert_eq!(result, 42);
}
