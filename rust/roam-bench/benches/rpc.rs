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

const PAYLOAD_SIZES: &[usize] = &[64, 256, 1024, 4096, 65536];
const STREAM_COUNTS: &[usize] = &[10, 100, 1000];

// ============================================================================
// roam
// ============================================================================

mod roam_bench {
    use moire::task::FutureExt;
    use roam_core::{BareConduit, Driver, acceptor, initiator, memory_link_pair};
    use roam_types::Parity;

    type MessageConduit = BareConduit<roam_types::MessageFamily, roam_core::MemoryLink>;

    #[roam::service]
    pub trait Bench {
        async fn add(&self, a: i32, b: i32) -> i32;
        async fn echo(&self, data: Vec<u8>) -> Vec<u8>;
        async fn generate(&self, count: u32, output: roam::Tx<i32>);
    }

    #[derive(Clone)]
    struct Handler;

    impl BenchServer for Handler {
        async fn add(&self, call: impl roam::Call<i32, core::convert::Infallible>, a: i32, b: i32) {
            call.ok(a + b).await;
        }

        async fn echo(
            &self,
            call: impl roam::Call<Vec<u8>, core::convert::Infallible>,
            data: Vec<u8>,
        ) {
            call.ok(data).await;
        }

        async fn generate(
            &self,
            call: impl roam::Call<(), core::convert::Infallible>,
            count: u32,
            output: roam::Tx<i32>,
        ) {
            for i in 0..count as i32 {
                output.send(i).await.unwrap();
            }
            output.close(Default::default()).await.unwrap();
            call.ok(()).await;
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

    pub async fn setup() -> BenchClient<roam_core::DriverCaller> {
        let (a, b) = memory_link_pair(64);
        let client_conduit: MessageConduit = BareConduit::new(a);
        let server_conduit: MessageConduit = BareConduit::new(b);

        let server_task = moire::task::spawn(
            async move {
                let (mut server_session, server_handle) = acceptor(server_conduit)
                    .establish()
                    .await
                    .expect("server handshake failed");
                let dispatcher = BenchDispatcher::new(Handler);
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

        BenchClient::new(caller)
    }
}

#[divan::bench]
fn roam_add(bencher: divan::Bencher) {
    let client = TOKIO.block_on(roam_bench::setup());
    bencher.bench_local(|| {
        TOKIO.block_on(async {
            let resp = client.add(3, 5).await.expect("roam add failed");
            divan::black_box(resp.ret);
        })
    });
}

#[divan::bench(args = PAYLOAD_SIZES)]
fn roam_echo(bencher: divan::Bencher, n: usize) {
    let client = TOKIO.block_on(roam_bench::setup());
    let payload = vec![42u8; n];
    bencher.bench_local(|| {
        TOKIO.block_on(async {
            let resp = client
                .echo(payload.clone())
                .await
                .expect("roam echo failed");
            divan::black_box(resp.ret.len());
        })
    });
}

#[divan::bench(args = STREAM_COUNTS)]
fn roam_stream(bencher: divan::Bencher, n: usize) {
    let client = TOKIO.block_on(roam_bench::setup());
    bencher.bench_local(|| {
        TOKIO.block_on(async {
            let (tx, mut rx) = roam::channel::<i32, 16>();
            let call = client.generate(n as u32, tx);
            let recv_task = tokio::spawn(async move {
                let mut count = 0u32;
                while let Ok(Some(_item)) = rx.recv().await {
                    count += 1;
                }
                count
            });
            call.await.expect("roam generate failed");
            let count = recv_task.await.unwrap();
            divan::black_box(count);
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
    pub trait Bench {
        async fn add(a: i32, b: i32) -> i32;
        async fn echo(data: Vec<u8>) -> Vec<u8>;
    }

    #[derive(Clone)]
    struct Handler;

    impl Bench for Handler {
        async fn add(self, _ctx: tarpc::context::Context, a: i32, b: i32) -> i32 {
            a + b
        }

        async fn echo(self, _ctx: tarpc::context::Context, data: Vec<u8>) -> Vec<u8> {
            data
        }
    }

    pub async fn setup() -> BenchClient {
        let (client_transport, server_transport) = tarpc::transport::channel::unbounded();

        tokio::spawn(async move {
            let incoming = tarpc::server::BaseChannel::with_defaults(server_transport)
                .execute(Handler.serve());
            tokio::pin!(incoming);
            while let Some(handler) = incoming.next().await {
                tokio::spawn(handler);
            }
        });

        BenchClient::new(tarpc::client::Config::default(), client_transport).spawn()
    }
}

#[divan::bench]
fn tarpc_add(bencher: divan::Bencher) {
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

#[divan::bench(args = PAYLOAD_SIZES)]
fn tarpc_echo(bencher: divan::Bencher, n: usize) {
    let client = TOKIO.block_on(tarpc_bench::setup());
    let payload = vec![42u8; n];
    bencher.bench_local(|| {
        TOKIO.block_on(async {
            let resp = client
                .echo(tarpc::context::current(), payload.clone())
                .await
                .expect("tarpc echo failed");
            divan::black_box(resp.len());
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
        AddRequest, AddResponse, EchoRequest, EchoResponse, GenerateRequest, Number,
        adder_client::AdderClient,
        adder_server::{Adder, AdderServer},
    };
    use tokio_stream::wrappers::ReceiverStream;
    use tonic::{
        Request, Response, Status,
        transport::{Endpoint, Server, Uri},
    };
    use tower::service_fn;

    #[derive(Default)]
    struct Handler;

    #[tonic::async_trait]
    impl Adder for Handler {
        async fn add(&self, request: Request<AddRequest>) -> Result<Response<AddResponse>, Status> {
            let req = request.into_inner();
            Ok(Response::new(AddResponse {
                result: req.a + req.b,
            }))
        }

        async fn echo(
            &self,
            request: Request<EchoRequest>,
        ) -> Result<Response<EchoResponse>, Status> {
            let req = request.into_inner();
            Ok(Response::new(EchoResponse { data: req.data }))
        }

        type GenerateStream = ReceiverStream<Result<Number, Status>>;

        async fn generate(
            &self,
            request: Request<GenerateRequest>,
        ) -> Result<Response<Self::GenerateStream>, Status> {
            let count = request.into_inner().count;
            let (tx, rx) = tokio::sync::mpsc::channel(128);
            tokio::spawn(async move {
                for i in 0..count {
                    if tx.send(Ok(Number { value: i })).await.is_err() {
                        break;
                    }
                }
            });
            Ok(Response::new(ReceiverStream::new(rx)))
        }
    }

    pub async fn setup() -> AdderClient<tonic::transport::Channel> {
        let (client, server) = tokio::io::duplex(1024);

        tokio::spawn(async move {
            Server::builder()
                .add_service(AdderServer::new(Handler))
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
fn tonic_add(bencher: divan::Bencher) {
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

#[divan::bench(args = PAYLOAD_SIZES)]
fn tonic_echo(bencher: divan::Bencher, n: usize) {
    let mut client = TOKIO.block_on(tonic_bench::setup());
    let payload = vec![42u8; n];
    bencher.bench_local(|| {
        TOKIO.block_on(async {
            let resp = client
                .echo(tonic_bench::pb::EchoRequest {
                    data: payload.clone(),
                })
                .await
                .expect("tonic echo failed")
                .into_inner();
            divan::black_box(resp.data.len());
        })
    });
}

#[divan::bench(args = STREAM_COUNTS)]
fn tonic_stream(bencher: divan::Bencher, n: usize) {
    let mut client = TOKIO.block_on(tonic_bench::setup());
    bencher.bench_local(|| {
        TOKIO.block_on(async {
            use tokio_stream::StreamExt;
            let mut stream = client
                .generate(tonic_bench::pb::GenerateRequest { count: n as i32 })
                .await
                .expect("tonic generate failed")
                .into_inner();
            let mut count = 0u32;
            while let Some(Ok(_)) = stream.next().await {
                count += 1;
            }
            divan::black_box(count);
        })
    });
}
