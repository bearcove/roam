//! Rust subject binary for the roam compliance suite.
//!
//! This demonstrates the minimal code needed to implement a roam service
//! using the roam-stream transport library.

use roam::session::{Rx, Tx};
use roam_stream::Server;

// Re-export types from spec_proto for use in generated code
pub use spec_proto::{Canvas, Color, Message, Person, Point, Rectangle, Shape};

// Include generated code (testbed::TestbedHandler, testbed::TestbedDispatcher, etc.)
include!(concat!(env!("OUT_DIR"), "/generated.rs"));

// Service implementation using generated TestbedHandler trait
#[derive(Clone)]
struct TestbedService;

impl testbed::TestbedHandler for TestbedService {
    // ========================================================================
    // Unary methods
    // ========================================================================

    async fn echo(&self, message: String) -> Result<String, testbed::RoamError<testbed::Never>> {
        Ok(message)
    }

    async fn reverse(&self, message: String) -> Result<String, testbed::RoamError<testbed::Never>> {
        Ok(message.chars().rev().collect())
    }

    // ========================================================================
    // Streaming methods
    // ========================================================================

    async fn sum(&self, mut numbers: Rx<i32>) -> Result<i64, testbed::RoamError<testbed::Never>> {
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
    ) -> Result<(), testbed::RoamError<testbed::Never>> {
        for i in 0..count as i32 {
            let _ = output.send(&i).await;
        }
        Ok(())
    }

    async fn transform(
        &self,
        mut input: Rx<String>,
        output: Tx<String>,
    ) -> Result<(), testbed::RoamError<testbed::Never>> {
        while let Some(s) = input.recv().await.ok().flatten() {
            let _ = output.send(&s).await;
        }
        Ok(())
    }
}

fn main() -> Result<(), String> {
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
