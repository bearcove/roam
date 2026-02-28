//! WebSocket peer server for testing TypeScript clients.
//!
//! This is a full roam implementation that TypeScript browser tests can
//! connect to. It uses the roam runtime (dispatcher, channels, etc.) to
//! provide a real roam peer for the TypeScript client to talk to.

use roam::{Rx, Tx};
use roam_core::{BareConduit, Driver, acceptor};
use roam_types::{MessageFamily, Parity};
use roam_websocket::WsLink;
use spec_proto::{Canvas, Color, LookupError, MathError, Message, Person, Point, Rectangle, Shape};
use spec_proto::{TestbedDispatcher, TestbedServer};
use std::env;
use tokio::net::TcpListener;

#[derive(Clone)]
struct TestbedService;

impl TestbedServer for TestbedService {
    async fn echo(
        &self,
        call: impl roam::Call<String, core::convert::Infallible>,
        message: String,
    ) {
        call.ok(message).await;
    }

    async fn reverse(
        &self,
        call: impl roam::Call<String, core::convert::Infallible>,
        message: String,
    ) {
        call.ok(message.chars().rev().collect()).await;
    }

    async fn divide(&self, call: impl roam::Call<i64, MathError>, dividend: i64, divisor: i64) {
        if divisor == 0 {
            call.err(MathError::DivisionByZero).await;
        } else {
            call.ok(dividend / divisor).await;
        }
    }

    async fn lookup(&self, call: impl roam::Call<Person, LookupError>, id: u32) {
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

    async fn sum(
        &self,
        call: impl roam::Call<i64, core::convert::Infallible>,
        mut numbers: Rx<i32>,
    ) {
        let mut total: i64 = 0;
        while let Ok(Some(n)) = numbers.recv().await {
            total += *n as i64;
        }
        call.ok(total).await;
    }

    async fn generate(
        &self,
        call: impl roam::Call<(), core::convert::Infallible>,
        count: u32,
        output: Tx<i32>,
    ) {
        for i in 0..count as i32 {
            let _ = output.send(i).await;
        }
        let _ = output.close(Default::default()).await;
        call.ok(()).await;
    }

    async fn transform(
        &self,
        call: impl roam::Call<(), core::convert::Infallible>,
        mut input: Rx<String>,
        output: Tx<String>,
    ) {
        while let Ok(Some(s)) = input.recv().await {
            let _ = output.send((*s).clone()).await;
        }
        let _ = output.close(Default::default()).await;
        call.ok(()).await;
    }

    async fn echo_point(
        &self,
        call: impl roam::Call<Point, core::convert::Infallible>,
        point: Point,
    ) {
        call.ok(point).await;
    }

    async fn create_person(
        &self,
        call: impl roam::Call<Person, core::convert::Infallible>,
        name: String,
        age: u8,
        email: Option<String>,
    ) {
        call.ok(Person { name, age, email }).await;
    }

    async fn rectangle_area(
        &self,
        call: impl roam::Call<f64, core::convert::Infallible>,
        rect: Rectangle,
    ) {
        let width = (rect.bottom_right.x - rect.top_left.x).abs() as f64;
        let height = (rect.bottom_right.y - rect.top_left.y).abs() as f64;
        call.ok(width * height).await;
    }

    async fn parse_color(
        &self,
        call: impl roam::Call<Option<Color>, core::convert::Infallible>,
        name: String,
    ) {
        let color = match name.to_lowercase().as_str() {
            "red" => Some(Color::Red),
            "green" => Some(Color::Green),
            "blue" => Some(Color::Blue),
            _ => None,
        };
        call.ok(color).await;
    }

    async fn shape_area(
        &self,
        call: impl roam::Call<f64, core::convert::Infallible>,
        shape: Shape,
    ) {
        let area = match shape {
            Shape::Circle { radius } => std::f64::consts::PI * radius * radius,
            Shape::Rectangle { width, height } => width * height,
            Shape::Point => 0.0,
        };
        call.ok(area).await;
    }

    async fn create_canvas(
        &self,
        call: impl roam::Call<Canvas, core::convert::Infallible>,
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

    async fn process_message(
        &self,
        call: impl roam::Call<Message, core::convert::Infallible>,
        msg: Message,
    ) {
        let result = match msg {
            Message::Text(text) => Message::Text(format!("Processed: {}", text)),
            Message::Number(n) => Message::Number(n * 2),
            Message::Data(data) => Message::Data(data.into_iter().rev().collect()),
        };
        call.ok(result).await;
    }

    async fn get_points(
        &self,
        call: impl roam::Call<Vec<Point>, core::convert::Infallible>,
        count: u32,
    ) {
        let points = (0..count as i32)
            .map(|i| Point { x: i, y: i * 2 })
            .collect();
        call.ok(points).await;
    }

    async fn swap_pair(
        &self,
        call: impl roam::Call<(String, i32), core::convert::Infallible>,
        pair: (i32, String),
    ) {
        call.ok((pair.1, pair.0)).await;
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

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("Accept error: {}", e);
                continue;
            }
        };

        eprintln!("New connection from {}", peer);

        tokio::spawn(async move {
            let ws_link = match WsLink::server(stream).await {
                Ok(link) => link,
                Err(e) => {
                    eprintln!("WebSocket handshake failed: {}", e);
                    return;
                }
            };

            let conduit: BareConduit<MessageFamily, _> = BareConduit::new(ws_link);
            let (mut session, handle, _sh) = match acceptor(conduit).establish().await {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("Session handshake failed: {:?}", e);
                    return;
                }
            };

            eprintln!("Connection established with {}", peer);

            let dispatcher = TestbedDispatcher::new(TestbedService);
            let mut driver = Driver::new(handle, dispatcher, Parity::Even);
            tokio::spawn(async move { session.run().await });
            driver.run().await;

            eprintln!("Connection closed: {}", peer);
        });
    }
}
