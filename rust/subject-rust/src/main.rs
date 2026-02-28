//! Rust subject binary for the roam compliance suite.

use roam::{Call, Rx, Tx};
use roam_core::{BareConduit, Driver, initiator};
use roam_stream::StreamLink;
use roam_types::{MessageFamily, Parity};
use spec_proto::{
    Canvas, Color, LookupError, MathError, Message, Person, Point, Rectangle, Shape,
    TestbedDispatcher, TestbedServer,
};
use std::convert::Infallible;
use tracing::{debug, error, info, instrument};

type TcpConduit = BareConduit<
    MessageFamily,
    StreamLink<tokio::net::tcp::OwnedReadHalf, tokio::net::tcp::OwnedWriteHalf>,
>;

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

    rt.block_on(run_server())
}

async fn run_server() -> Result<(), String> {
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
