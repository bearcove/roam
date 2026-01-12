//! Integration tests for the HTTP bridge.
//!
//! This tests the full flow:
//! 1. Start a roam server with Testbed service
//! 2. Start an HTTP bridge connected to it
//! 3. Make HTTP requests to the bridge
//! 4. Verify responses

use std::net::SocketAddr;

use axum::Router;
use roam_http_bridge::{BridgeRouter, GenericBridgeService};
use roam_stream::{Connector, HandshakeConfig, NoDispatcher, accept, connect};
use spec_proto::{Testbed, TestbedDispatcher, testbed_service_detail};
use tokio::net::{TcpListener, TcpStream};

/// Simple Testbed implementation for testing.
#[derive(Clone)]
struct TestbedImpl;

impl Testbed for TestbedImpl {
    async fn echo(
        &self,
        message: String,
    ) -> Result<String, roam_session::RoamError<roam_session::Never>> {
        Ok(message)
    }

    async fn reverse(
        &self,
        message: String,
    ) -> Result<String, roam_session::RoamError<roam_session::Never>> {
        Ok(message.chars().rev().collect())
    }

    async fn sum(
        &self,
        mut numbers: roam_session::Rx<i32>,
    ) -> Result<i64, roam_session::RoamError<roam_session::Never>> {
        let mut total: i64 = 0;
        while let Some(n) = numbers.recv().await.ok().flatten() {
            total += n as i64;
        }
        Ok(total)
    }

    async fn generate(
        &self,
        count: u32,
        output: roam_session::Tx<i32>,
    ) -> Result<(), roam_session::RoamError<roam_session::Never>> {
        for i in 0..count as i32 {
            let _ = output.send(&i).await;
        }
        Ok(())
    }

    async fn transform(
        &self,
        mut input: roam_session::Rx<String>,
        output: roam_session::Tx<String>,
    ) -> Result<(), roam_session::RoamError<roam_session::Never>> {
        while let Some(s) = input.recv().await.ok().flatten() {
            let _ = output.send(&s.to_uppercase()).await;
        }
        Ok(())
    }

    async fn echo_point(
        &self,
        point: spec_proto::Point,
    ) -> Result<spec_proto::Point, roam_session::RoamError<roam_session::Never>> {
        Ok(point)
    }

    async fn create_person(
        &self,
        name: String,
        age: u8,
        email: Option<String>,
    ) -> Result<spec_proto::Person, roam_session::RoamError<roam_session::Never>> {
        Ok(spec_proto::Person { name, age, email })
    }

    async fn rectangle_area(
        &self,
        rect: spec_proto::Rectangle,
    ) -> Result<f64, roam_session::RoamError<roam_session::Never>> {
        let width = (rect.bottom_right.x - rect.top_left.x).abs() as f64;
        let height = (rect.bottom_right.y - rect.top_left.y).abs() as f64;
        Ok(width * height)
    }

    async fn parse_color(
        &self,
        name: String,
    ) -> Result<Option<spec_proto::Color>, roam_session::RoamError<roam_session::Never>> {
        let color = match name.to_lowercase().as_str() {
            "red" => Some(spec_proto::Color::Red),
            "green" => Some(spec_proto::Color::Green),
            "blue" => Some(spec_proto::Color::Blue),
            _ => None,
        };
        Ok(color)
    }

    async fn shape_area(
        &self,
        shape: spec_proto::Shape,
    ) -> Result<f64, roam_session::RoamError<roam_session::Never>> {
        let area = match shape {
            spec_proto::Shape::Circle { radius } => std::f64::consts::PI * radius * radius,
            spec_proto::Shape::Rectangle { width, height } => width * height,
            spec_proto::Shape::Point => 0.0,
        };
        Ok(area)
    }

    async fn create_canvas(
        &self,
        name: String,
        shapes: Vec<spec_proto::Shape>,
        background: spec_proto::Color,
    ) -> Result<spec_proto::Canvas, roam_session::RoamError<roam_session::Never>> {
        Ok(spec_proto::Canvas {
            name,
            shapes,
            background,
        })
    }

    async fn process_message(
        &self,
        msg: spec_proto::Message,
    ) -> Result<spec_proto::Message, roam_session::RoamError<roam_session::Never>> {
        Ok(msg)
    }

    async fn get_points(
        &self,
        count: u32,
    ) -> Result<Vec<spec_proto::Point>, roam_session::RoamError<roam_session::Never>> {
        let points = (0..count as i32)
            .map(|i| spec_proto::Point { x: i, y: i * 2 })
            .collect();
        Ok(points)
    }

    async fn swap_pair(
        &self,
        pair: (i32, String),
    ) -> Result<(String, i32), roam_session::RoamError<roam_session::Never>> {
        Ok((pair.1, pair.0))
    }
}

/// Start a roam server and return the address.
async fn start_roam_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let dispatcher = TestbedDispatcher::new(TestbedImpl);

            tokio::spawn(async move {
                let (handle, driver) = accept(stream, HandshakeConfig::default(), dispatcher)
                    .await
                    .unwrap();
                let _ = handle;
                driver.run().await;
            });
        }
    });

    (addr, handle)
}

/// Connector for the bridge client.
struct BridgeConnector {
    addr: SocketAddr,
}

impl Connector for BridgeConnector {
    type Transport = TcpStream;

    async fn connect(&self) -> std::io::Result<TcpStream> {
        TcpStream::connect(self.addr).await
    }
}

/// Connect to the roam server and return a connection handle.
async fn connect_to_roam(addr: SocketAddr) -> roam_stream::Client<BridgeConnector, NoDispatcher> {
    let connector = BridgeConnector { addr };
    connect(connector, HandshakeConfig::default(), NoDispatcher)
}

/// Start the HTTP bridge server.
async fn start_bridge_server(
    roam_client: roam_stream::Client<BridgeConnector, NoDispatcher>,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    // Get a handle from the client
    let handle = roam_client.handle().await.unwrap();

    // Leak the service detail to get a 'static reference
    let detail: &'static _ = Box::leak(Box::new(testbed_service_detail()));
    let service = GenericBridgeService::new(handle, detail);

    let bridge_router = BridgeRouter::new().service(service).build();

    let app = Router::new().nest("/api", bridge_router);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

#[tokio::test]
async fn test_echo_via_http_bridge() {
    // 1. Start roam server
    let (roam_addr, _server_handle) = start_roam_server().await;

    // Give server time to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 2. Connect to roam server
    let roam_client = connect_to_roam(roam_addr).await;

    // 3. Start HTTP bridge
    let (bridge_addr, _bridge_handle) = start_bridge_server(roam_client).await;

    // Give bridge time to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 4. Make HTTP request
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/Testbed/echo", bridge_addr))
        .header("Content-Type", "application/json")
        .body(r#"["hello world"]"#)
        .send()
        .await
        .unwrap();

    let status = response.status();
    let body_text = response.text().await.unwrap();

    if status != 200 {
        eprintln!("Response status: {}", status);
        eprintln!("Response body: {}", body_text);
    }

    assert_eq!(status, 200, "Body was: {}", body_text);

    let body: String = serde_json::from_str(&body_text).unwrap();
    assert_eq!(body, "hello world");
}

#[tokio::test]
async fn test_reverse_via_http_bridge() {
    let (roam_addr, _) = start_roam_server().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let roam_client = connect_to_roam(roam_addr).await;
    let (bridge_addr, _) = start_bridge_server(roam_client).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/Testbed/reverse", bridge_addr))
        .header("Content-Type", "application/json")
        .body(r#"["hello"]"#)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let body: String = response.json().await.unwrap();
    assert_eq!(body, "olleh");
}

#[tokio::test]
async fn test_echo_point_via_http_bridge() {
    let (roam_addr, _) = start_roam_server().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let roam_client = connect_to_roam(roam_addr).await;
    let (bridge_addr, _) = start_bridge_server(roam_client).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/Testbed/echo_point", bridge_addr))
        .header("Content-Type", "application/json")
        .body(r#"[{"x": 10, "y": 20}]"#)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["x"], 10);
    assert_eq!(body["y"], 20);
}

#[tokio::test]
async fn test_streaming_method_rejected() {
    let (roam_addr, _) = start_roam_server().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let roam_client = connect_to_roam(roam_addr).await;
    let (bridge_addr, _) = start_bridge_server(roam_client).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/Testbed/sum", bridge_addr))
        .header("Content-Type", "application/json")
        .body(r#"[]"#)
        .send()
        .await
        .unwrap();

    // r[bridge.json.channels-forbidden] - should be rejected with 400
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn test_unknown_method() {
    let (roam_addr, _) = start_roam_server().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let roam_client = connect_to_roam(roam_addr).await;
    let (bridge_addr, _) = start_bridge_server(roam_client).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/Testbed/nonexistent", bridge_addr))
        .header("Content-Type", "application/json")
        .body(r#"[]"#)
        .send()
        .await
        .unwrap();

    // Unknown method returns 200 with error JSON (it's a BridgeError for now)
    assert_eq!(response.status(), 200);
}
