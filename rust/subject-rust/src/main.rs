//! Rust subject binary for the roam compliance suite.
//!
//! This demonstrates the minimal code needed to implement a roam service
//! using the roam-stream transport library.

use roam::session::{Rx, Tx};
use roam_stream::Server;
use tracing::{debug, error, info, instrument};

// Re-export types from spec_proto for use in generated code
pub use spec_proto::{Canvas, Color, Message, Person, Point, Rectangle, Shape};

// Include generated code (testbed::Testbed, testbed::TestbedDispatcher, etc.)
include!(concat!(env!("OUT_DIR"), "/generated.rs"));

// Service implementation using generated Testbed trait
#[derive(Clone)]
struct TestbedService;

impl testbed::Testbed for TestbedService {
    // ========================================================================
    // Unary methods
    // ========================================================================

    #[instrument(skip(self))]
    async fn echo(&self, message: String) -> Result<String, testbed::RoamError<testbed::Never>> {
        info!("echo called");
        Ok(message)
    }

    #[instrument(skip(self))]
    async fn reverse(&self, message: String) -> Result<String, testbed::RoamError<testbed::Never>> {
        info!("reverse called");
        Ok(message.chars().rev().collect())
    }

    // ========================================================================
    // Streaming methods
    // ========================================================================

    #[instrument(skip(self, numbers))]
    async fn sum(&self, mut numbers: Rx<i32>) -> Result<i64, testbed::RoamError<testbed::Never>> {
        info!("sum called, starting to receive numbers");
        let mut total: i64 = 0;
        while let Some(n) = numbers.recv().await.ok().flatten() {
            debug!(n, total, "received number");
            total += n as i64;
        }
        info!(total, "sum complete");
        Ok(total)
    }

    #[instrument(skip(self, output))]
    async fn generate(
        &self,
        count: u32,
        output: Tx<i32>,
    ) -> Result<(), testbed::RoamError<testbed::Never>> {
        info!(count, "generate called");
        for i in 0..count as i32 {
            debug!(i, "sending value");
            match output.send(&i).await {
                Ok(()) => debug!(i, "sent OK"),
                Err(e) => error!(i, ?e, "send failed"),
            }
        }
        info!("generate complete");
        Ok(())
    }

    #[instrument(skip(self, input, output))]
    async fn transform(
        &self,
        mut input: Rx<String>,
        output: Tx<String>,
    ) -> Result<(), testbed::RoamError<testbed::Never>> {
        info!("transform called");
        while let Some(s) = input.recv().await.ok().flatten() {
            debug!(?s, "transforming");
            let _ = output.send(&s).await;
        }
        info!("transform complete");
        Ok(())
    }

    // ========================================================================
    // Complex type methods
    // ========================================================================

    async fn echo_point(&self, point: Point) -> Result<Point, testbed::RoamError<testbed::Never>> {
        Ok(point)
    }

    async fn create_person(
        &self,
        name: String,
        age: u8,
        email: Option<String>,
    ) -> Result<Person, testbed::RoamError<testbed::Never>> {
        Ok(Person { name, age, email })
    }

    async fn rectangle_area(
        &self,
        rect: Rectangle,
    ) -> Result<f64, testbed::RoamError<testbed::Never>> {
        let width = (rect.bottom_right.x - rect.top_left.x).abs() as f64;
        let height = (rect.bottom_right.y - rect.top_left.y).abs() as f64;
        Ok(width * height)
    }

    async fn parse_color(
        &self,
        name: String,
    ) -> Result<Option<Color>, testbed::RoamError<testbed::Never>> {
        let color = match name.to_lowercase().as_str() {
            "red" => Some(Color::Red),
            "green" => Some(Color::Green),
            "blue" => Some(Color::Blue),
            _ => None,
        };
        Ok(color)
    }

    async fn shape_area(&self, shape: Shape) -> Result<f64, testbed::RoamError<testbed::Never>> {
        let area = match shape {
            Shape::Circle { radius } => std::f64::consts::PI * radius * radius,
            Shape::Rectangle { width, height } => width * height,
            Shape::Point => 0.0,
        };
        Ok(area)
    }

    async fn create_canvas(
        &self,
        name: String,
        shapes: Vec<Shape>,
        background: Color,
    ) -> Result<Canvas, testbed::RoamError<testbed::Never>> {
        Ok(Canvas {
            name,
            shapes,
            background,
        })
    }

    async fn process_message(
        &self,
        msg: Message,
    ) -> Result<Message, testbed::RoamError<testbed::Never>> {
        // Echo the message back with some transformation
        let response = match msg {
            Message::Text(s) => Message::Text(format!("processed: {s}")),
            Message::Number(n) => Message::Number(n * 2),
            Message::Data(d) => Message::Data(d.into_iter().rev().collect()),
        };
        Ok(response)
    }

    async fn get_points(
        &self,
        count: u32,
    ) -> Result<Vec<Point>, testbed::RoamError<testbed::Never>> {
        let points = (0..count as i32)
            .map(|i| Point { x: i, y: i * 2 })
            .collect();
        Ok(points)
    }

    async fn swap_pair(
        &self,
        pair: (i32, String),
    ) -> Result<(String, i32), testbed::RoamError<testbed::Never>> {
        Ok((pair.1, pair.0))
    }
}

fn main() -> Result<(), String> {
    // Initialize tracing subscriber (respects RUST_LOG env var)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    info!("subject-rust starting");

    // Manual runtime (avoid tokio-macros / syn).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create tokio runtime: {e}"))?;

    rt.block_on(async {
        let server = Server::new();
        // Use generated dispatcher with our service implementation
        let dispatcher = testbed::TestbedDispatcher::new(TestbedService);
        server
            .run_subject(&dispatcher)
            .await
            .map_err(|e| format!("{e:?}"))
    })
}
