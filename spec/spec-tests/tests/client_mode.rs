//! Client-mode compliance tests.
//!
//! These tests run the spec-tests harness as a wire-level server and spawn
//! the subject in client mode. This validates that the generated client code
//! works correctly against a minimal protocol implementation.
//!
//! The harness does NOT use any roam runtime types - it implements the server
//! at the wire level using only roam_wire messages.

use spec_tests::harness::{run_async, wire_server};
use spec_tests::testbed::method_id;

fn method_ids() -> wire_server::MethodIds {
    wire_server::MethodIds {
        echo: method_id::echo().0,
        reverse: method_id::reverse().0,
        sum: method_id::sum().0,
        generate: method_id::generate().0,
        transform: method_id::transform().0,
        shape_area: method_id::shape_area().0,
        create_canvas: method_id::create_canvas().0,
        process_message: method_id::process_message().0,
    }
}

// r[verify call.initiate] - Generated client can make RPC calls
#[test]
fn client_mode_echo() {
    run_async(async { wire_server::run("echo", &method_ids()).await }).unwrap();
}

// r[verify channeling.type] - Generated client can send channel data
// r[verify channeling.type] - Client pushes data via Rx channel
#[test]
fn client_mode_sum() {
    run_async(async { wire_server::run("sum", &method_ids()).await }).unwrap();
}

// r[verify channeling.type] - Generated client can receive channel data
// r[verify channeling.type] - Server pushes data via Tx channel
#[test]
fn client_mode_generate() {
    run_async(async { wire_server::run("generate", &method_ids()).await }).unwrap();
}

// r[verify call.request.payload-encoding] - Generated client encodes enum args in request payload.
// r[verify call.response.encoding] - Generated client decodes enum-typed responses.
#[test]
fn client_mode_shape_area() {
    run_async(async { wire_server::run("shape_area", &method_ids()).await }).unwrap();
}

// r[verify call.request.payload-encoding] - Generated client encodes mixed args including Vec<enum>.
// r[verify call.response.encoding] - Generated client decodes nested enum payloads in responses.
#[test]
fn client_mode_create_canvas() {
    run_async(async { wire_server::run("create_canvas", &method_ids()).await }).unwrap();
}

// r[verify call.request.payload-encoding] - Generated client encodes newtype enum variants.
// r[verify call.response.encoding] - Generated client decodes newtype enum variants.
#[test]
fn client_mode_process_message() {
    run_async(async { wire_server::run("process_message", &method_ids()).await }).unwrap();
}
