//! WebSocket server for browser testing.
//!
//! Serves the Calculator service over WebSocket on port 9000.

use codegen_test_consumer::calculator::{
    CalculatorDispatcher, CalculatorHandler, Never, RoamError,
};
use roam::session::{Rx, Tx};
use roam_stream::Hello;
use roam_websocket::{WsTransport, ws_accept};
use std::env;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

#[derive(Clone)]
struct Calculator;

impl CalculatorHandler for Calculator {
    async fn add(&self, a: i32, b: i32) -> Result<i32, RoamError<Never>> {
        Ok(a + b)
    }

    async fn multiply(&self, a: i32, b: i32) -> Result<i32, RoamError<Never>> {
        Ok(a * b)
    }

    async fn sum_stream(&self, mut numbers: Rx<i32>) -> Result<i64, RoamError<Never>> {
        let mut sum: i64 = 0;
        while let Some(n) = numbers.recv().await.ok().flatten() {
            sum += n as i64;
        }
        Ok(sum)
    }

    async fn range(&self, count: u32, output: Tx<u32>) -> Result<(), RoamError<Never>> {
        for i in 0..count {
            let _ = output.send(&i).await;
        }
        Ok(())
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
            let dispatcher = CalculatorDispatcher::new(Calculator);
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
