use std::sync::LazyLock;

use tokio::runtime::Runtime;

static TOKIO: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
});

fn main() {
    divan::main();
}

// ============================================================================
// roam
// ============================================================================

mod roam_bench {
    use moire::task::FutureExt;
    use roam_core::{BareConduit, Driver, acceptor, initiator, memory_link_pair};
    use roam_types::Parity;

    type MessageConduit = BareConduit<roam_types::MessageFamily, roam_core::MemoryLink>;

    #[roam::service]
    pub trait Adder {
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

    impl roam_types::Handler<roam_core::DriverReplySink> for NoopHandler {
        fn handle(
            &self,
            _call: roam_types::SelfRef<roam_types::RequestCall<'static>>,
            _reply: roam_core::DriverReplySink,
        ) -> impl std::future::Future<Output = ()> + Send + '_ {
            async {}
        }
    }

    pub async fn setup() -> AdderClient<roam_core::DriverCaller> {
        let (a, b) = memory_link_pair(64);
        let client_conduit: MessageConduit = BareConduit::new(a);
        let server_conduit: MessageConduit = BareConduit::new(b);

        let server_task = moire::task::spawn(
            async move {
                let (mut server_session, server_handle) = acceptor(server_conduit)
                    .establish()
                    .await
                    .expect("server handshake failed");
                let dispatcher = AdderDispatcher::new(MyAdder);
                let mut server_driver = Driver::new(server_handle, dispatcher, Parity::Even);
                moire::task::spawn(
                    async move { server_session.run().await }.named("server_session"),
                );
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

        AdderClient::new(caller)
    }
}

#[divan::bench]
fn roam_memory(bencher: divan::Bencher) {
    let client = TOKIO.block_on(roam_bench::setup());
    bencher.bench_local(|| {
        TOKIO.block_on(async {
            let resp = client.add(3, 5).await.expect("roam add failed");
            divan::black_box(resp.ret);
        })
    });
}

// ============================================================================
// tarpc
// ============================================================================

mod tarpc_bench {
    use futures_util::StreamExt;
    use tarpc::server::Channel;

    #[tarpc::service]
    pub trait Adder {
        async fn add(a: i32, b: i32) -> i32;
    }

    #[derive(Clone)]
    struct MyAdder;

    impl Adder for MyAdder {
        async fn add(self, _ctx: tarpc::context::Context, a: i32, b: i32) -> i32 {
            a + b
        }
    }

    pub async fn setup() -> AdderClient {
        let (client_transport, server_transport) = tarpc::transport::channel::unbounded();

        tokio::spawn(async move {
            let incoming = tarpc::server::BaseChannel::with_defaults(server_transport)
                .execute(MyAdder.serve());
            tokio::pin!(incoming);
            while let Some(handler) = incoming.next().await {
                tokio::spawn(handler);
            }
        });

        AdderClient::new(tarpc::client::Config::default(), client_transport).spawn()
    }
}

#[divan::bench]
fn tarpc_memory(bencher: divan::Bencher) {
    let client = TOKIO.block_on(tarpc_bench::setup());
    bencher.bench_local(|| {
        TOKIO.block_on(async {
            let resp = client
                .add(tarpc::context::current(), 3, 5)
                .await
                .expect("tarpc add failed");
            divan::black_box(resp);
        })
    });
}

// ============================================================================
// tonic (in-memory via DuplexStream)
// ============================================================================

mod tonic_bench {
    pub mod pb {
        tonic::include_proto!("adder");
    }

    use hyper_util::rt::TokioIo;
    use pb::{
        AddRequest, AddResponse,
        adder_client::AdderClient,
        adder_server::{Adder, AdderServer},
    };
    use tonic::{
        Request, Response, Status,
        transport::{Endpoint, Server, Uri},
    };
    use tower::service_fn;

    #[derive(Default)]
    struct MyAdder;

    #[tonic::async_trait]
    impl Adder for MyAdder {
        async fn add(&self, request: Request<AddRequest>) -> Result<Response<AddResponse>, Status> {
            let req = request.into_inner();
            Ok(Response::new(AddResponse {
                result: req.a + req.b,
            }))
        }
    }

    pub async fn setup() -> AdderClient<tonic::transport::Channel> {
        let (client, server) = tokio::io::duplex(1024);

        tokio::spawn(async move {
            Server::builder()
                .add_service(AdderServer::new(MyAdder))
                .serve_with_incoming(tokio_stream::once(Ok::<_, std::io::Error>(server)))
                .await
                .unwrap();
        });

        let mut client = Some(client);
        let channel = Endpoint::try_from("http://[::]:50051")
            .unwrap()
            .connect_with_connector(service_fn(move |_: Uri| {
                let client = client.take();
                async move {
                    client
                        .map(TokioIo::new)
                        .ok_or_else(|| std::io::Error::other("client already taken"))
                }
            }))
            .await
            .unwrap();

        AdderClient::new(channel)
    }
}

#[divan::bench]
fn tonic_memory(bencher: divan::Bencher) {
    let mut client = TOKIO.block_on(tonic_bench::setup());
    bencher.bench_local(|| {
        TOKIO.block_on(async {
            let resp = client
                .add(tonic_bench::pb::AddRequest { a: 3, b: 5 })
                .await
                .expect("tonic add failed")
                .into_inner();
            divan::black_box(resp.result);
        })
    });
}
