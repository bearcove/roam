use moire::task::FutureExt;
use roam_types::Parity;

use crate::session::{acceptor, initiator};
use crate::{BareConduit, Driver, memory_link_pair};

type MessageConduit = BareConduit<roam_types::MessageFamily, crate::MemoryLink>;

fn message_conduit_pair() -> (MessageConduit, MessageConduit) {
    let (a, b) = memory_link_pair(64);
    (BareConduit::new(a), BareConduit::new(b))
}

// Define a service using the macro. The generated code references `roam::*`
// paths, which the facade crate re-exports.
#[roam::service]
trait Adder {
    /// Add two numbers.
    async fn add(&self, a: i32, b: i32) -> i32;
}

/// Our handler implementation.
#[derive(Clone)]
struct MyAdder;

impl AdderServer for MyAdder {
    async fn add(&self, call: impl roam::Call<i32, core::convert::Infallible>, a: i32, b: i32) {
        call.ok(a + b).await;
    }
}

/// No-op handler for the client side (it doesn't receive incoming calls).
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
async fn adder_service_macro_end_to_end() {
    let (client_conduit, server_conduit) = message_conduit_pair();

    // Server side: use the macro-generated dispatcher
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

    // Client side: use the macro-generated client
    let (mut client_session, client_handle) = initiator(client_conduit)
        .establish()
        .await
        .expect("client handshake failed");
    let mut client_driver = Driver::new(client_handle, NoopHandler, Parity::Odd);
    let caller = client_driver.caller();
    moire::task::spawn(async move { client_session.run().await }.named("client_session"));
    moire::task::spawn(async move { client_driver.run().await }.named("client_driver"));

    server_task.await.expect("server setup failed");

    // Use the typed client
    let client = AdderClient::new(caller);
    let response = client.add(3, 5).await.expect("add call should succeed");
    assert_eq!(response.ret, 8);

    let response = client.add(100, -42).await.expect("add call should succeed");
    assert_eq!(response.ret, 58);
}
