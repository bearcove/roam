//! Rust subject binary for the roam compliance suite.

use roam::{Call, Rx, Tx};
use roam_core::{BareConduit, Driver, initiator};
use roam_shm::bootstrap::{BootstrapStatus, encode_request};
use roam_shm::guest_link_from_raw;
use roam_shm::segment::Segment;
use roam_stream::StreamLink;
use roam_types::{MessageFamily, Parity};
use spec_proto::{
    Canvas, Color, LookupError, MathError, Message, Person, Point, Rectangle, Shape,
    TestbedDispatcher, TestbedServer,
};
use std::convert::Infallible;
use std::os::fd::AsRawFd;
use tracing::{debug, error, info, instrument};

type TcpConduit = BareConduit<
    MessageFamily,
    StreamLink<tokio::net::tcp::OwnedReadHalf, tokio::net::tcp::OwnedWriteHalf>,
>;
type ShmConduit = BareConduit<MessageFamily, roam_shm::ShmLink>;

#[derive(Clone)]
struct TestbedService;

impl TestbedServer for TestbedService {
    #[instrument(skip(self, call))]
    async fn echo(&self, call: impl Call<String, Infallible>, message: String) {
        info!("echo called");
        call.ok(message).await;
    }

    #[instrument(skip(self, call))]
    async fn reverse(&self, call: impl Call<String, Infallible>, message: String) {
        info!("reverse called");
        call.ok(message.chars().rev().collect()).await;
    }

    #[instrument(skip(self, call))]
    async fn divide(&self, call: impl Call<i64, MathError>, dividend: i64, divisor: i64) {
        info!("divide called");
        if divisor == 0 {
            call.err(MathError::DivisionByZero).await;
        } else {
            call.ok(dividend / divisor).await;
        }
    }

    #[instrument(skip(self, call))]
    async fn lookup(&self, call: impl Call<Person, LookupError>, id: u32) {
        info!("lookup called");
        match id {
            1 => {
                call.ok(Person {
                    name: "Alice".to_string(),
                    age: 30,
                    email: Some("alice@example.com".to_string()),
                })
                .await
            }
            2 => {
                call.ok(Person {
                    name: "Bob".to_string(),
                    age: 25,
                    email: None,
                })
                .await
            }
            3 => {
                call.ok(Person {
                    name: "Charlie".to_string(),
                    age: 35,
                    email: Some("charlie@example.com".to_string()),
                })
                .await
            }
            _ => call.err(LookupError::NotFound).await,
        }
    }

    #[instrument(skip(self, call, numbers))]
    async fn sum(&self, call: impl Call<i64, Infallible>, mut numbers: Rx<i32>) {
        info!("sum called");
        let mut total: i64 = 0;
        while let Ok(Some(n)) = numbers.recv().await {
            debug!(n = *n, total, "received number");
            total += *n as i64;
        }
        info!(total, "sum complete");
        call.ok(total).await;
    }

    #[instrument(skip(self, call, output))]
    async fn generate(&self, call: impl Call<(), Infallible>, count: u32, output: Tx<i32>) {
        info!(count, "generate called");
        for i in 0..count as i32 {
            debug!(i, "sending value");
            if let Err(e) = output.send(i).await {
                error!(i, ?e, "send failed");
                break;
            }
        }
        output.close(Default::default()).await.ok();
        call.ok(()).await;
    }

    #[instrument(skip(self, call, input, output))]
    async fn transform(
        &self,
        call: impl Call<(), Infallible>,
        mut input: Rx<String>,
        output: Tx<String>,
    ) {
        info!("transform called");
        while let Ok(Some(s)) = input.recv().await {
            debug!(s = ?*s, "transforming");
            let _ = output.send(s.clone()).await;
        }
        output.close(Default::default()).await.ok();
        call.ok(()).await;
    }

    async fn echo_point(&self, call: impl Call<Point, Infallible>, point: Point) {
        call.ok(point).await;
    }

    async fn create_person(
        &self,
        call: impl Call<Person, Infallible>,
        name: String,
        age: u8,
        email: Option<String>,
    ) {
        call.ok(Person { name, age, email }).await;
    }

    async fn rectangle_area(&self, call: impl Call<f64, Infallible>, rect: Rectangle) {
        let width = (rect.bottom_right.x - rect.top_left.x).abs() as f64;
        let height = (rect.bottom_right.y - rect.top_left.y).abs() as f64;
        call.ok(width * height).await;
    }

    async fn parse_color(&self, call: impl Call<Option<Color>, Infallible>, name: String) {
        let color = match name.to_lowercase().as_str() {
            "red" => Some(Color::Red),
            "green" => Some(Color::Green),
            "blue" => Some(Color::Blue),
            _ => None,
        };
        call.ok(color).await;
    }

    async fn shape_area(&self, call: impl Call<f64, Infallible>, shape: Shape) {
        let area = match shape {
            Shape::Circle { radius } => std::f64::consts::PI * radius * radius,
            Shape::Rectangle { width, height } => width * height,
            Shape::Point => 0.0,
        };
        call.ok(area).await;
    }

    async fn create_canvas(
        &self,
        call: impl Call<Canvas, Infallible>,
        name: String,
        shapes: Vec<Shape>,
        background: Color,
    ) {
        call.ok(Canvas {
            name,
            shapes,
            background,
        })
        .await;
    }

    async fn process_message(&self, call: impl Call<Message, Infallible>, msg: Message) {
        let response = match msg {
            Message::Text(s) => Message::Text(format!("processed: {s}")),
            Message::Number(n) => Message::Number(n * 2),
            Message::Data(d) => Message::Data(d.into_iter().rev().collect()),
        };
        call.ok(response).await;
    }

    async fn get_points(&self, call: impl Call<Vec<Point>, Infallible>, count: u32) {
        let points = (0..count as i32)
            .map(|i| Point { x: i, y: i * 2 })
            .collect();
        call.ok(points).await;
    }

    async fn swap_pair(&self, call: impl Call<(String, i32), Infallible>, pair: (i32, String)) {
        call.ok((pair.1, pair.0)).await;
    }
}

fn main() -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create tokio runtime: {e}"))?;

    let mode = std::env::var("SUBJECT_MODE").unwrap_or_else(|_| "server".to_string());
    match mode.as_str() {
        "server" => rt.block_on(connect_and_serve()),
        "shm-server" => rt.block_on(connect_and_serve_shm()),
        other => Err(format!("unknown SUBJECT_MODE: {other}")),
    }
}

async fn connect_and_serve() -> Result<(), String> {
    let addr = std::env::var("PEER_ADDR").map_err(|_| "PEER_ADDR env var not set".to_string())?;
    info!("connecting to {addr}");

    let stream = tokio::net::TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    stream.set_nodelay(true).unwrap();

    let conduit: TcpConduit = BareConduit::new(StreamLink::tcp(stream));
    let (mut session, handle, _sh) = initiator(conduit)
        .establish()
        .await
        .map_err(|e| format!("handshake failed: {e}"))?;

    let dispatcher = TestbedDispatcher::new(TestbedService);
    let mut driver = Driver::new(handle, dispatcher, Parity::Odd);

    moire::task::spawn(async move { session.run().await });
    driver.run().await;
    Ok(())
}

async fn connect_and_serve_shm() -> Result<(), String> {
    let control_sock = std::env::var("SHM_CONTROL_SOCK")
        .map_err(|_| "SHM_CONTROL_SOCK env var not set".to_string())?;
    let sid = std::env::var("SHM_SESSION_ID")
        .map_err(|_| "SHM_SESSION_ID env var not set".to_string())?;
    let mmap_tx_fd: i32 = std::env::var("SHM_MMAP_TX_FD")
        .map_err(|_| "SHM_MMAP_TX_FD env var not set".to_string())?
        .parse()
        .map_err(|e| format!("invalid SHM_MMAP_TX_FD: {e}"))?;

    let mut stream = std::os::unix::net::UnixStream::connect(&control_sock)
        .map_err(|e| format!("connect bootstrap socket: {e}"))?;
    let request = encode_request(sid.as_bytes()).map_err(|e| format!("encode request: {e}"))?;
    std::io::Write::write_all(&mut stream, &request)
        .map_err(|e| format!("send bootstrap request: {e}"))?;

    let received = shm_primitives::bootstrap::recv_response_unix(stream.as_raw_fd())
        .map_err(|e| format!("recv bootstrap response: {e}"))?;
    if received.response.status != BootstrapStatus::Success {
        return Err(format!(
            "bootstrap failed: status={:?}, payload={}",
            received.response.status,
            String::from_utf8_lossy(&received.response.payload)
        ));
    }
    let fds = received
        .fds
        .ok_or_else(|| "missing bootstrap success fds".to_string())?;
    let hub_path = std::str::from_utf8(&received.response.payload)
        .map_err(|e| format!("bootstrap payload is not utf-8 path: {e}"))?;
    let segment = std::sync::Arc::new(
        Segment::attach(std::path::Path::new(hub_path))
            .map_err(|e| format!("attach segment at {hub_path}: {e}"))?,
    );
    let peer_id = shm_primitives::PeerId::new(received.response.peer_id as u8)
        .ok_or_else(|| format!("invalid peer id {}", received.response.peer_id))?;

    use std::os::fd::IntoRawFd;
    let doorbell_fd = fds.doorbell_fd.into_raw_fd();
    let mmap_rx_fd = fds
        .mmap_control_fd
        .ok_or_else(|| "missing mmap control fd in bootstrap success".to_string())?
        .into_raw_fd();

    let link =
        unsafe { guest_link_from_raw(segment, peer_id, doorbell_fd, mmap_rx_fd, mmap_tx_fd, true) }
            .map_err(|e| format!("guest_link_from_raw: {e}"))?;
    let conduit: ShmConduit = BareConduit::new(link);

    let (mut session, handle, _sh) = initiator(conduit)
        .establish()
        .await
        .map_err(|e| format!("handshake failed: {e}"))?;

    let dispatcher = TestbedDispatcher::new(TestbedService);
    let mut driver = Driver::new(handle, dispatcher, Parity::Odd);

    moire::task::spawn(async move { session.run().await });
    driver.run().await;
    Ok(())
}
