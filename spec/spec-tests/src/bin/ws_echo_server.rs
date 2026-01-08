//! WebSocket server for browser testing.
//!
//! Serves Testbed, Complex, and Streaming services over WebSocket for cross-language testing.

use roam::session::{Rx, Tx};
use roam_stream::{Hello, RoutedDispatcher};
use roam_websocket::{WsTransport, ws_accept};
use spec_proto::{Canvas, Color, Message, Person, Point, Rectangle, Shape};
use spec_tests::complex::{ComplexHandler, Never as ComplexNever, RoamError as ComplexRoamError};
use spec_tests::streaming::{
    Never as StreamingNever, RoamError as StreamingRoamError, StreamingHandler,
};
use spec_tests::testbed::{Never as TestbedNever, RoamError as TestbedRoamError, TestbedHandler};
use spec_tests::{complex, streaming, testbed};
use std::env;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

// Testbed method IDs (from generated code)
const TESTBED_METHODS: &[u64] = &[
    testbed::method_id::ECHO,
    testbed::method_id::REVERSE,
    testbed::method_id::SUM,
    testbed::method_id::GENERATE,
    testbed::method_id::TRANSFORM,
];

// Streaming method IDs (all streaming methods are handled by StreamingDispatcher)
const STREAMING_METHODS: &[u64] = &[
    streaming::method_id::SUM,
    streaming::method_id::RANGE,
    streaming::method_id::PIPE,
    streaming::method_id::STATS,
];

// Service implementation using generated TestbedHandler trait
#[derive(Clone)]
struct TestbedService;

impl TestbedHandler for TestbedService {
    async fn echo(&self, message: String) -> Result<String, TestbedRoamError<TestbedNever>> {
        Ok(message)
    }

    async fn reverse(&self, message: String) -> Result<String, TestbedRoamError<TestbedNever>> {
        Ok(message.chars().rev().collect())
    }

    async fn sum(&self, mut numbers: Rx<i32>) -> Result<i64, TestbedRoamError<TestbedNever>> {
        let mut total: i64 = 0;
        while let Some(n) = numbers.recv().await.ok().flatten() {
            total += n as i64;
        }
        Ok(total)
    }

    async fn generate(
        &self,
        count: u32,
        output: Tx<i32>,
    ) -> Result<(), TestbedRoamError<TestbedNever>> {
        for i in 0..count as i32 {
            let _ = output.send(&i).await;
        }
        Ok(())
    }

    async fn transform(
        &self,
        mut input: Rx<String>,
        output: Tx<String>,
    ) -> Result<(), TestbedRoamError<TestbedNever>> {
        while let Some(s) = input.recv().await.ok().flatten() {
            let _ = output.send(&s).await;
        }
        Ok(())
    }
}

// Service implementation using generated ComplexHandler trait
#[derive(Clone)]
struct ComplexService;

impl ComplexHandler for ComplexService {
    async fn echo_point(&self, point: Point) -> Result<Point, ComplexRoamError<ComplexNever>> {
        Ok(point)
    }

    async fn create_person(
        &self,
        name: String,
        age: u8,
        email: Option<String>,
    ) -> Result<Person, ComplexRoamError<ComplexNever>> {
        Ok(Person { name, age, email })
    }

    async fn rectangle_area(&self, rect: Rectangle) -> Result<f64, ComplexRoamError<ComplexNever>> {
        let width = (rect.bottom_right.x - rect.top_left.x).abs() as f64;
        let height = (rect.bottom_right.y - rect.top_left.y).abs() as f64;
        Ok(width * height)
    }

    async fn parse_color(
        &self,
        name: String,
    ) -> Result<Option<Color>, ComplexRoamError<ComplexNever>> {
        match name.to_lowercase().as_str() {
            "red" => Ok(Some(Color::Red)),
            "green" => Ok(Some(Color::Green)),
            "blue" => Ok(Some(Color::Blue)),
            _ => Ok(None),
        }
    }

    async fn shape_area(&self, shape: Shape) -> Result<f64, ComplexRoamError<ComplexNever>> {
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
    ) -> Result<Canvas, ComplexRoamError<ComplexNever>> {
        Ok(Canvas {
            name,
            shapes,
            background,
        })
    }

    async fn process_message(
        &self,
        msg: Message,
    ) -> Result<Message, ComplexRoamError<ComplexNever>> {
        match msg {
            Message::Text(text) => Ok(Message::Text(format!("Processed: {}", text))),
            Message::Number(n) => Ok(Message::Number(n * 2)),
            Message::Data(data) => Ok(Message::Data(data.into_iter().rev().collect())),
        }
    }

    async fn get_points(&self, count: u32) -> Result<Vec<Point>, ComplexRoamError<ComplexNever>> {
        Ok((0..count as i32)
            .map(|i| Point { x: i, y: i * 2 })
            .collect())
    }

    async fn swap_pair(
        &self,
        pair: (i32, String),
    ) -> Result<(String, i32), ComplexRoamError<ComplexNever>> {
        Ok((pair.1, pair.0))
    }
}

// Service implementation using generated StreamingHandler trait
#[derive(Clone)]
struct StreamingService;

impl StreamingHandler for StreamingService {
    async fn sum(&self, mut numbers: Rx<i32>) -> Result<i64, StreamingRoamError<StreamingNever>> {
        let mut total: i64 = 0;
        while let Some(n) = numbers.recv().await.ok().flatten() {
            total += n as i64;
        }
        Ok(total)
    }

    async fn range(
        &self,
        count: u32,
        output: Tx<u32>,
    ) -> Result<(), StreamingRoamError<StreamingNever>> {
        for i in 0..count {
            let _ = output.send(&i).await;
        }
        Ok(())
    }

    async fn pipe(
        &self,
        mut input: Rx<String>,
        output: Tx<String>,
    ) -> Result<(), StreamingRoamError<StreamingNever>> {
        while let Some(s) = input.recv().await.ok().flatten() {
            let _ = output.send(&s).await;
        }
        Ok(())
    }

    async fn stats(
        &self,
        mut numbers: Rx<i32>,
    ) -> Result<(i64, u64, f64), StreamingRoamError<StreamingNever>> {
        let mut sum: i64 = 0;
        let mut count: u64 = 0;
        while let Some(n) = numbers.recv().await.ok().flatten() {
            sum += n as i64;
            count += 1;
        }
        let avg = if count > 0 {
            sum as f64 / count as f64
        } else {
            0.0
        };
        Ok((sum, count, avg))
    }
}

#[tokio::main]
async fn main() {
    let port = env::var("WS_PORT").unwrap_or_else(|_| "9000".to_string());
    let addr = format!("127.0.0.1:{}", port);

    let listener = TcpListener::bind(&addr).await.unwrap();
    eprintln!("WebSocket server listening on ws://{}", addr);

    // Print port on stdout for Playwright to parse
    println!("{}", port);

    let hello = Hello::V1 {
        max_payload_size: 1024 * 1024,
        initial_stream_credit: 64 * 1024,
    };

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("Accept error: {}", e);
                continue;
            }
        };

        eprintln!("New connection from {}", peer);

        let ws_stream = match accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("WebSocket handshake failed: {}", e);
                continue;
            }
        };

        let transport = WsTransport::new(ws_stream);
        let hello = hello.clone();

        tokio::spawn(async move {
            // Combine Testbed, Complex, and Streaming dispatchers using nested RoutedDispatcher
            let testbed_dispatcher = testbed::TestbedDispatcher::new(TestbedService);
            let complex_dispatcher = complex::ComplexDispatcher::new(ComplexService);
            let streaming_dispatcher = streaming::StreamingDispatcher::new(StreamingService);

            // First combine Testbed and Complex
            let testbed_complex =
                RoutedDispatcher::new(testbed_dispatcher, complex_dispatcher, TESTBED_METHODS);
            // Then add Streaming
            let dispatcher =
                RoutedDispatcher::new(streaming_dispatcher, testbed_complex, STREAMING_METHODS);

            match ws_accept(transport, hello).await {
                Ok(mut conn) => {
                    eprintln!("Connection established with {}", peer);
                    if let Err(e) = conn.run(&dispatcher).await {
                        eprintln!("Connection error: {:?}", e);
                    }
                    eprintln!("Connection closed: {}", peer);
                }
                Err(e) => {
                    eprintln!("Hello exchange failed: {:?}", e);
                }
            }
        });
    }
}
