//! In-process, high-volume RPC benchmark for roam-session.
//!
//! Usage:
//! - `cargo run -p roam-session --example in_process_bench --release -- --iterations 200000`
//! - `ROAM_DISABLE_TYPE_PLAN_CACHE=1 cargo run -p roam-session --example in_process_bench --release -- --iterations 200000`
//!
//! Sampling example (samply):
//! - `cargo samply record -p roam-session --example in_process_bench --release -- --iterations 200000`
//! - `ROAM_DISABLE_TYPE_PLAN_CACHE=1 cargo samply record -p roam-session --example in_process_bench --release -- --iterations 200000`

use std::io;
use std::time::{Duration, Instant};

use roam_session::{
    HandshakeConfig, MessageTransport, NoDispatcher, accept_framed, initiate_framed,
};
use roam_wire::Message;
use spec_proto::{
    Canvas, Color, LookupError, MathError, Message as BenchMessage, Person, Point, Rectangle,
    Shape, Testbed, TestbedClient, TestbedDispatcher,
};
use tokio::sync::mpsc;

struct InMemoryTransport {
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    last_decoded: Vec<u8>,
}

fn in_memory_transport_pair(buffer: usize) -> (InMemoryTransport, InMemoryTransport) {
    let (a_to_b_tx, a_to_b_rx) = mpsc::channel(buffer);
    let (b_to_a_tx, b_to_a_rx) = mpsc::channel(buffer);

    let a = InMemoryTransport {
        tx: a_to_b_tx,
        rx: b_to_a_rx,
        last_decoded: Vec::new(),
    };
    let b = InMemoryTransport {
        tx: b_to_a_tx,
        rx: a_to_b_rx,
        last_decoded: Vec::new(),
    };

    (a, b)
}

impl MessageTransport for InMemoryTransport {
    async fn send(&mut self, msg: &Message) -> io::Result<()> {
        self.tx
            .send(msg.clone())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "peer disconnected"))
    }

    async fn recv_timeout(&mut self, timeout: Duration) -> io::Result<Option<Message>> {
        match tokio::time::timeout(timeout, self.rx.recv()).await {
            Ok(msg) => Ok(msg),
            Err(_) => Ok(None),
        }
    }

    async fn recv(&mut self) -> io::Result<Option<Message>> {
        Ok(self.rx.recv().await)
    }

    fn last_decoded(&self) -> &[u8] {
        &self.last_decoded
    }
}

#[derive(Clone)]
struct BenchService;

impl Testbed for BenchService {
    async fn echo(&self, _cx: &roam_session::Context, message: String) -> String {
        message
    }

    async fn reverse(&self, _cx: &roam_session::Context, message: String) -> String {
        message.chars().rev().collect()
    }

    async fn divide(
        &self,
        _cx: &roam_session::Context,
        dividend: i64,
        divisor: i64,
    ) -> Result<i64, MathError> {
        if divisor == 0 {
            Err(MathError::DivisionByZero)
        } else {
            Ok(dividend / divisor)
        }
    }

    async fn lookup(&self, _cx: &roam_session::Context, id: u32) -> Result<Person, LookupError> {
        if id == 1 {
            Ok(Person {
                name: "Alice".to_string(),
                age: 30,
                email: Some("alice@example.com".to_string()),
            })
        } else {
            Err(LookupError::NotFound)
        }
    }

    async fn sum(&self, _cx: &roam_session::Context, mut numbers: roam_session::Rx<i32>) -> i64 {
        let mut total = 0i64;
        while let Ok(Some(n)) = numbers.recv().await {
            total += n as i64;
        }
        total
    }

    async fn generate(
        &self,
        _cx: &roam_session::Context,
        count: u32,
        output: roam_session::Tx<i32>,
    ) {
        for i in 0..count as i32 {
            let _ = output.send(&i).await;
        }
    }

    async fn transform(
        &self,
        _cx: &roam_session::Context,
        mut input: roam_session::Rx<String>,
        output: roam_session::Tx<String>,
    ) {
        while let Ok(Some(s)) = input.recv().await {
            let _ = output.send(&s).await;
        }
    }

    async fn echo_point(&self, _cx: &roam_session::Context, point: Point) -> Point {
        point
    }

    async fn create_person(
        &self,
        _cx: &roam_session::Context,
        name: String,
        age: u8,
        email: Option<String>,
    ) -> Person {
        Person { name, age, email }
    }

    async fn rectangle_area(&self, _cx: &roam_session::Context, rect: Rectangle) -> f64 {
        let width = (rect.bottom_right.x - rect.top_left.x).abs() as f64;
        let height = (rect.bottom_right.y - rect.top_left.y).abs() as f64;
        width * height
    }

    async fn parse_color(&self, _cx: &roam_session::Context, name: String) -> Option<Color> {
        match name.to_lowercase().as_str() {
            "red" => Some(Color::Red),
            "green" => Some(Color::Green),
            "blue" => Some(Color::Blue),
            _ => None,
        }
    }

    async fn shape_area(&self, _cx: &roam_session::Context, shape: Shape) -> f64 {
        match shape {
            Shape::Circle { radius } => std::f64::consts::PI * radius * radius,
            Shape::Rectangle { width, height } => width * height,
            Shape::Point => 0.0,
        }
    }

    async fn create_canvas(
        &self,
        _cx: &roam_session::Context,
        name: String,
        shapes: Vec<Shape>,
        background: Color,
    ) -> Canvas {
        Canvas {
            name,
            shapes,
            background,
        }
    }

    async fn process_message(
        &self,
        _cx: &roam_session::Context,
        msg: BenchMessage,
    ) -> BenchMessage {
        msg
    }

    async fn get_points(&self, _cx: &roam_session::Context, count: u32) -> Vec<Point> {
        (0..count as i32)
            .map(|i| Point { x: i, y: i * 2 })
            .collect()
    }

    async fn swap_pair(&self, _cx: &roam_session::Context, pair: (i32, String)) -> (String, i32) {
        (pair.1, pair.0)
    }
}

struct Config {
    iterations: usize,
    payload_bytes: usize,
    warmup: usize,
}

fn parse_args() -> Result<Config, String> {
    let mut iterations = 200_000usize;
    let mut payload_bytes = 128usize;
    let mut warmup = 2_000usize;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--iterations" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "--iterations needs a value".to_string())?;
                iterations = raw
                    .parse::<usize>()
                    .map_err(|e| format!("invalid --iterations: {e}"))?;
            }
            "--payload-bytes" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "--payload-bytes needs a value".to_string())?;
                payload_bytes = raw
                    .parse::<usize>()
                    .map_err(|e| format!("invalid --payload-bytes: {e}"))?;
            }
            "--warmup" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "--warmup needs a value".to_string())?;
                warmup = raw
                    .parse::<usize>()
                    .map_err(|e| format!("invalid --warmup: {e}"))?;
            }
            _ => {
                return Err(format!(
                    "unknown arg: {arg}. expected --iterations N --payload-bytes N --warmup N"
                ));
            }
        }
    }

    Ok(Config {
        iterations,
        payload_bytes,
        warmup,
    })
}

async fn run(config: Config) -> Result<(), String> {
    let (client_transport, server_transport) = in_memory_transport_pair(8192);
    let dispatcher = TestbedDispatcher::new(BenchService);

    let client_fut = initiate_framed(client_transport, HandshakeConfig::default(), NoDispatcher);
    let server_fut = accept_framed(server_transport, HandshakeConfig::default(), dispatcher);

    let (client_setup, server_setup) = tokio::try_join!(client_fut, server_fut)
        .map_err(|e| format!("failed to establish in-memory connection: {e}"))?;

    let (client_handle, _incoming_client, client_driver) = client_setup;
    let (_server_handle, _incoming_server, server_driver) = server_setup;

    let client_driver_task = tokio::spawn(async move { client_driver.run().await });
    let server_driver_task = tokio::spawn(async move { server_driver.run().await });

    let client = TestbedClient::new(client_handle);
    let payload = "x".repeat(config.payload_bytes);

    for _ in 0..config.warmup {
        let echoed = client
            .echo(payload.clone())
            .await
            .map_err(|e| format!("warmup call failed: {e}"))?;
        if echoed.len() != payload.len() {
            return Err("warmup response had wrong length".to_string());
        }
    }

    let started = Instant::now();
    for _ in 0..config.iterations {
        let echoed = client
            .echo(payload.clone())
            .await
            .map_err(|e| format!("benchmark call failed: {e}"))?;
        if echoed.len() != payload.len() {
            return Err("benchmark response had wrong length".to_string());
        }
    }
    let elapsed = started.elapsed();

    let seconds = elapsed.as_secs_f64();
    let rps = config.iterations as f64 / seconds;
    let us_per_call = elapsed.as_micros() as f64 / config.iterations as f64;

    println!(
        "bench complete: iterations={} payload_bytes={} warmup={} elapsed={:.3}s calls_per_sec={:.0} avg_us_per_call={:.2}",
        config.iterations, config.payload_bytes, config.warmup, seconds, rps, us_per_call
    );
    println!("tip: set ROAM_DISABLE_TYPE_PLAN_CACHE=1 to profile uncached type-plan preparation");

    client_driver_task.abort();
    server_driver_task.abort();

    Ok(())
}

fn main() -> Result<(), String> {
    let config = parse_args()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create tokio runtime: {e}"))?;

    rt.block_on(run(config))
}
