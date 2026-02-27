use moire::task::FutureExt;
use roam_shm::ShmLink;
use roam_shm::varslot::SizeClassConfig;
use roam_types::Parity;

use crate::session::{acceptor, initiator};
use crate::{BareConduit, Driver};

type MessageConduit = BareConduit<roam_types::MessageFamily, ShmLink>;

fn message_conduit_pair() -> (MessageConduit, MessageConduit) {
    let classes = [SizeClassConfig {
        slot_size: 4096,
        slot_count: 16,
    }];
    let (a, b) = ShmLink::heap_pair(1 << 16, 1 << 20, 256, &classes);
    (BareConduit::new(a), BareConduit::new(b))
}

#[roam::service]
trait Adder {
    async fn add(&self, a: i32, b: i32) -> i32;
}

#[derive(Clone)]
struct MyAdder;

impl AdderServer for MyAdder {
    async fn add(&self, call: impl roam::Call<i32, core::convert::Infallible>, a: i32, b: i32) {
        call.ok(a + b).await;
    }
}

struct NoopHandler;

impl roam_types::Handler<crate::DriverReplySink> for NoopHandler {
    fn handle(
        &self,
        _call: roam_types::SelfRef<roam_types::RequestCall<'static>>,
        _reply: crate::DriverReplySink,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        async {}
    }
}

#[tokio::test]
async fn adder_service_macro_end_to_end_over_shm() {
    let (client_conduit, server_conduit) = message_conduit_pair();

    let server_task = moire::task::spawn(
        async move {
            let (mut server_session, server_handle) = acceptor(server_conduit)
                .establish()
                .await
                .expect("server handshake failed");
            let dispatcher = AdderDispatcher::new(MyAdder);
            let mut server_driver = Driver::new(server_handle, dispatcher, Parity::Even);
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

    let client = AdderClient::new(caller);
    let response = client.add(3, 5).await.expect("add call should succeed");
    assert_eq!(response.ret, 8);

    let response = client.add(100, -42).await.expect("add call should succeed");
    assert_eq!(response.ret, 58);
}
