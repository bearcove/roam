# Rust Codegen Architecture

This document describes the architecture for generating Rust client/server code from roam service definitions.

## The Big Picture

**Goal**: A single `Connection` type that handles everything, so using roam services is as simple as:

```rust
// Client side - with generated code
let conn = Connection::connect(&addr).await?;
let client = CalculatorClient::new(conn);

let result = client.add(2, 3).await?;  // Just works!

// Streaming
let (tx, rx) = roam::channel::<i32>();
let result_fut = client.sum_stream(tx);
tx.send(&1).await?;
tx.send(&2).await?;
drop(tx);
let sum: i64 = result_fut.await?;
```

```rust
// Server side - with generated code
let conn = Connection::accept(&listener).await?;
let dispatcher = CalculatorDispatcher::new(MyHandler);
conn.run(&dispatcher).await?;
```

**Current state** (what we're replacing): Manual message construction, serialization, request ID tracking:

```rust
// This is painful and error-prone!
let payload = facet_postcard::to_vec(&(2i32, 3i32)).unwrap();
conn.io().send(&Message::Request {
    request_id: 1,  // manual tracking
    method_id: method_id::ADD,
    metadata: vec![],
    payload,
}).await.unwrap();

let resp = conn.io().recv_timeout(...).await.unwrap().unwrap();
let result: CallResult<i32, Never> = facet_postcard::from_slice(&payload).unwrap();
```

**The `Connection` type orchestrates**:
1. **Request/response multiplexing** - track in-flight calls, route responses
2. **Request ID allocation** - automatic, no manual tracking
3. **Stream ID allocation and binding** - walk args with `Poke`, find `Tx<T>`/`Rx<T>`, set their `stream_id` fields
4. **Serialization** - uses facet (Tx/Rx serialize as `u64` via proxy)
5. **Symmetric design** - same type works for client AND server

**Why Tx/Rx need special Facet treatment** (see experiments below):
- `Connection::call_raw(method_id, poke)` receives a `Poke` to the arguments
- It walks the arg structure, finds any `Tx<T>`/`Rx<T>` handles
- It allocates stream IDs and **pokes them into** the `stream_id` field
- Then it serializes - Tx/Rx become just `u64` on the wire via `#[facet(proxy = u64)]`

## Implementation Status

### ✅ Completed

**Tx/Rx Facet Implementation** (`roam-session`):
- `Tx<T>` and `Rx<T>` now use `#[derive(Facet)]` with `#[facet(proxy = u64)]`
- `stream_id` field is public and pokeable
- `sender`/`receiver` fields use `#[facet(opaque)]`
- `TryFrom<&Tx<T>> for u64` and `TryFrom<u64> for Tx<T>` for serialization
- Type parameter `T` is exposed for codegen introspection
- All existing tests pass

**ConnectionHandle** (`roam-session`):
- `ConnectionHandle` - cloneable handle for making outgoing calls
- `call_raw(method_id, payload)` - make raw RPC call with pre-serialized payload
- `new_tx<T>()` / `new_rx<T>()` - allocate streams
- Stream registry integration for routing stream data
- `HandleCommand` enum for communication with driver
- `CallError` error type

### 🚧 In Progress

**Connection type** - single symmetric type for client AND server:
- Need `call_raw(method_id, poke)` that accepts `Poke` instead of pre-serialized payload
- Walk args with Poke to find Tx/Rx and bind stream IDs
- Serialize via facet after binding

### 📋 TODO

**Generated client code** using Connection:
```rust
// Goal: This should just work
let client = CalculatorClient::new(conn);
let result = client.add(2, 3).await?;
```

**Update tests** to use new API instead of manual message construction

## Overview

All language targets (including Rust) generate their client/server code through `roam-codegen` running in a `build.rs` script. The proc-macro (`roam-macros`) is **metadata-only** — it parses the trait syntax and emits a single `service_detail()` function.

## Architectural Principles

1. **Transport-agnostic**: Generated code works with any transport implementing the required traits
2. **Session reuse**: Generated code builds on `roam-session` primitives, not reimplementing connection logic
3. **Full spec support**: Unary calls, client→server streams, server→client streams, bidirectional, complex types
4. **Caller-POV schema**: `Tx` means caller sends, `Rx` means caller receives; handlers invert this

## Component Roles

### `roam-macros` (proc-macro) — Minimal Metadata Emitter

The `#[service]` macro parses the trait syntax and emits **only** the service detail function:

```rust
// INPUT: trait is the DSL for defining services
#[roam::service]
trait Calculator {
    /// Add two numbers.
    async fn add(&self, a: i32, b: i32) -> i32;
    
    /// Sum a stream of numbers.
    async fn sum_stream(&self, numbers: Tx<i32>) -> i64;
}

// OUTPUT: macro emits ONLY this function
pub fn calculator_service_detail() -> roam::schema::ServiceDetail {
    roam::schema::ServiceDetail {
        name: "Calculator".into(),
        methods: vec![
            roam::schema::MethodDetail {
                service_name: "Calculator".into(),
                method_name: "add".into(),
                args: vec![
                    roam::schema::ArgDetail { name: "a".into(), ty: <i32 as Facet>::SHAPE },
                    roam::schema::ArgDetail { name: "b".into(), ty: <i32 as Facet>::SHAPE },
                ],
                return_type: <i32 as Facet>::SHAPE,
                doc: Some("Add two numbers.".into()),
            },
            // ... more methods
        ],
        doc: None,
    }
}
```

**Explicitly NOT emitted by the macro:**
- The original trait definition (it's input DSL only)
- `{service}_method_ids()` — computed by codegen
- `{Service}Client<C>` — generated by codegen
- `{service}_dispatch_*()` — generated by codegen
- Any serialization/deserialization logic

### `roam-codegen/targets/rust.rs` — Client/Server Generator

Generates complete, production-ready code from `ServiceDetail`:

```rust
// @generated by roam-codegen

pub mod calculator {
    pub use ::roam::session::{Tx, Rx, StreamId, RoamError, CallResult, Never};

    /// Method IDs for this service (computed from ServiceDetail).
    pub mod method_id {
        pub const ADD: u64 = 0x...;
        pub const SUM_STREAM: u64 = 0x...;
    }

    /// Client for Calculator service.
    pub struct CalculatorClient<C> {
        caller: C,
        stream_registry: StreamRegistry,
        stream_allocator: StreamIdAllocator,
    }

    impl<C: UnaryCaller + StreamingCaller> CalculatorClient<C> {
        pub fn new(caller: C, role: Role) -> Self { ... }

        pub async fn add(&mut self, a: i32, b: i32) -> CallResult<i32> { ... }

        pub async fn sum_stream(&mut self) -> CallResult<(Tx<i32>, oneshot::Receiver<i64>)> { ... }
    }

    /// Handler trait for Calculator service.
    pub trait CalculatorHandler: Send + Sync {
        async fn add(&self, a: i32, b: i32) -> Result<i32, RoamError>;
        async fn sum_stream(&self, numbers: Rx<i32>) -> Result<i64, RoamError>;
    }

    /// Dispatcher that routes calls to a handler.
    pub struct CalculatorDispatcher<H> { ... }

    impl<H: CalculatorHandler + 'static> ServiceDispatcher for CalculatorDispatcher<H> { ... }
}
```

### `roam-session` — Core Abstractions

The session layer provides transport-agnostic primitives:

```rust
/// Minimal async caller for unary requests.
pub trait UnaryCaller {
    type Error;
    async fn call_unary(&mut self, method_id: u64, payload: Vec<u8>) -> Result<Frame, Self::Error>;
}

/// Caller that supports streaming (channel setup).
pub trait StreamingCaller: UnaryCaller {
    async fn call_streaming(
        &mut self,
        method_id: u64,
        payload: Vec<u8>,
        registry: &mut StreamRegistry,
    ) -> Result<Frame, Self::Error>;
}

/// Server-side dispatcher interface.
pub trait ServiceDispatcher: Send + Sync {
    fn is_streaming(&self, method_id: u64) -> bool;

    fn dispatch_unary(
        &self,
        method_id: u64,
        payload: &[u8],
    ) -> impl Future<Output = Result<Vec<u8>, String>> + Send;

    fn dispatch_streaming(
        &self,
        method_id: u64,
        payload: Vec<u8>,
        registry: &mut StreamRegistry,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send + 'static>>;
}

/// Stream types (from caller's perspective)
pub struct Tx<T> { ... }  // Caller sends T
pub struct Rx<T> { ... }  // Caller receives T

/// Manages stream lifecycle
pub struct StreamRegistry { ... }
```

### Transport Crates

Transport crates implement the caller/session traits:

- `roam-tcp` — TCP transport (implements `UnaryCaller`, `StreamingCaller`)
- `roam-ws` — WebSocket transport
- `roam-shm` — Shared memory transport (zero-copy)

## Streaming Architecture

### Stream Direction (Caller POV)

The schema always describes streams from the **caller's perspective**:

| In Schema | Caller Does | Handler Does |
|-----------|-------------|--------------|
| `Tx<T>` | Sends T values | Receives T values (gets `Rx<T>`) |
| `Rx<T>` | Receives T values | Sends T values (gets `Tx<T>`) |

### Stream Lifecycle

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  CLIENT (Caller)                              SERVER (Handler)              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  1. Client allocates stream IDs (odd for initiator)                         │
│     stream_id = allocator.next()                                            │
│                                                                             │
│  2. Client sends Request with stream IDs in payload                         │
│     ─────────────────────────────────────────────────────────────────────►  │
│                                                                             │
│  3. Server registers streams in its StreamRegistry                          │
│     - For Tx<T> args → register as incoming (server receives)              │
│     - For Rx<T> args → register as outgoing (server sends)                 │
│                                                                             │
│  4. Server creates inverted handles and calls handler                       │
│     handler.sum_stream(Rx::new(stream_id, registry))                       │
│                                                                             │
│  5. Data flows bidirectionally via StreamData frames                        │
│     ◄════════════════════════════════════════════════════════════════════►  │
│                                                                             │
│  6. Server sends Response with return value                                 │
│     ◄─────────────────────────────────────────────────────────────────────  │
│                                                                             │
│  7. Streams close (StreamClose frames) when handles drop                    │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Generated Streaming Code

For a method like:

```rust
async fn sum_stream(&self, numbers: Tx<i32>) -> i64;
```

**Client side (generated):**

```rust
pub async fn sum_stream(&mut self) -> CallResult<(Tx<i32>, oneshot::Receiver<i64>)> {
    // 1. Allocate stream ID for the Tx<i32>
    let numbers_stream_id = self.stream_allocator.next();

    // 2. Register outgoing stream (client sends)
    let (tx_sender, tx_handle) = self.stream_registry.register_outgoing(numbers_stream_id);
    let numbers_tx = Tx::new(numbers_stream_id, tx_sender);

    // 3. Encode stream IDs into request payload
    let payload = facet_postcard::to_vec(&(numbers_stream_id,))?;

    // 4. Send request
    let response = self.caller.call_streaming(
        method_id::SUM_STREAM,
        payload,
        &mut self.stream_registry,
    ).await?;

    // 5. Spawn task to receive result after streams complete
    let (result_tx, result_rx) = oneshot::channel();
    tokio::spawn(async move {
        let result: i64 = facet_postcard::from_slice(response.payload_bytes())?;
        let _ = result_tx.send(result);
    });

    Ok((numbers_tx, result_rx))
}
```

**Handler side (generated dispatcher):**

```rust
fn dispatch_streaming(
    &self,
    method_id: u64,
    payload: Vec<u8>,
    registry: &mut StreamRegistry,
) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send + 'static>> {
    match method_id {
        method_id::SUM_STREAM => {
            // 1. Decode stream ID from payload
            let (numbers_stream_id,): (StreamId,) = facet_postcard::from_slice(&payload)?;

            // 2. Register as INCOMING (handler receives what caller sends)
            let rx_receiver = registry.register_incoming(numbers_stream_id);
            let numbers_rx = Rx::new(numbers_stream_id, rx_receiver);

            // 3. Clone handler for 'static future
            let handler = self.handler.clone();

            Box::pin(async move {
                // 4. Call handler with inverted type
                let result = handler.sum_stream(numbers_rx).await?;

                // 5. Encode response
                Ok(facet_postcard::to_vec(&result)?)
            })
        }
        _ => { ... }
    }
}
```

## Usage Pattern

### Proto Crate (service definitions)

```rust
// my-proto/src/lib.rs
use roam::service;
use roam::session::{Tx, Rx};

#[service]
trait MyService {
    async fn unary_call(&self, input: String) -> String;
    async fn client_stream(&self, data: Tx<Chunk>) -> Summary;
    async fn server_stream(&self, query: Query, results: Rx<Item>);
    async fn bidi_stream(&self, requests: Tx<Request>, responses: Rx<Response>);
}

// Macro emits only: my_service_service_detail()
```

### Consumer Crate (build.rs)

```rust
// my-consumer/build.rs
use std::{env, fs, path::Path};

fn main() {
    let detail = my_proto::my_service_service_detail();

    let code = roam_codegen::targets::rust::generate_service_with_options(
        &detail,
        &roam_codegen::targets::rust::RustCodegenOptions {
            tracing: true,
        },
    );

    let out_dir = env::var("OUT_DIR").unwrap();
    fs::write(Path::new(&out_dir).join("generated.rs"), code).unwrap();

    println!("cargo::rerun-if-changed=build.rs");
}
```

### Consumer Crate (usage)

```rust
// my-consumer/src/lib.rs
include!(concat!(env!("OUT_DIR"), "/generated.rs"));

use my_service::{MyServiceClient, MyServiceHandler, MyServiceDispatcher};

// Client usage
async fn client_example(tcp: TcpSession) {
    let mut client = MyServiceClient::new(tcp, Role::Initiator);

    // Unary call
    let result = client.unary_call("hello".into()).await?;

    // Streaming call
    let (tx, result_rx) = client.client_stream().await?;
    tx.send(Chunk { data: vec![1, 2, 3] }).await?;
    tx.send(Chunk { data: vec![4, 5, 6] }).await?;
    drop(tx); // Close stream
    let summary = result_rx.await?;
}

// Server usage
struct MyHandler;

impl MyServiceHandler for MyHandler {
    async fn unary_call(&self, input: String) -> Result<String, RoamError> {
        Ok(format!("Echo: {input}"))
    }

    async fn client_stream(&self, mut data: Rx<Chunk>) -> Result<Summary, RoamError> {
        let mut total = 0;
        while let Some(chunk) = data.recv().await? {
            total += chunk.data.len();
        }
        Ok(Summary { total_bytes: total })
    }
}

async fn server_example(tcp: TcpSession) {
    let dispatcher = MyServiceDispatcher::new(MyHandler);
    // Use dispatcher with a server loop
}
```

## Migration Path

### Phase 1: Remove TypeDetail (HANDOFF.md)

1. Update `roam-macros` to use `<T as Facet>::SHAPE` directly
2. Remove `roam-reflect` crate
3. Verify codegen tests pass

### Phase 2: Strip Proc-Macro to Metadata-Only

1. Remove trait re-emission (trait is input DSL only)
2. Remove `generate_method_ids()` — codegen computes these
3. Remove `generate_client_methods()` — codegen generates client
4. Remove `generate_dispatch_arms()` — codegen generates dispatcher
5. Keep ONLY: `{service}_service_detail()` function

The macro should be ~50 lines after this, down from ~600.

### Phase 3: Expand rust.rs Codegen

1. **Method ID computation**
   - Move from proc-macro to codegen
   - Compute from `ServiceDetail` using `roam_hash::method_id()`

2. **Client struct generation**
   - Wraps any `C: UnaryCaller + StreamingCaller`
   - Holds `StreamRegistry` for channel management
   - Holds `StreamIdAllocator` for ID allocation

3. **Full unary support**
   - Argument encoding with `facet-postcard`
   - Response decoding with zero-copy where possible
   - Proper error handling with `CallResult<T, E>`

4. **Full streaming support**
   - Stream ID allocation (odd for initiator, even for acceptor)
   - Registry integration for incoming/outgoing streams
   - Tx/Rx inversion for handler side
   - Proper cleanup on drop

5. **Handler trait + Dispatcher**
   - `impl ServiceDispatcher` for transport integration
   - Support for `RoutedDispatcher` (multiple services on one session)
   - Tracing integration (optional)

6. **Tests**
   - Round-trip tests with mock transport
   - Streaming lifecycle tests
   - Error case tests

## Key Types Reference

### roam-session

| Type | Purpose |
|------|---------|
| `UnaryCaller` | Trait for making unary RPC calls |
| `StreamingCaller` | Trait for making streaming RPC calls |
| `ServiceDispatcher` | Trait for server-side dispatch |
| `Tx<T>` | Send handle for caller-to-handler stream |
| `Rx<T>` | Receive handle for handler-to-caller stream |
| `StreamRegistry` | Manages active streams in a session |
| `StreamIdAllocator` | Allocates stream IDs (odd/even by role) |
| `Role` | `Initiator` (odd IDs) or `Acceptor` (even IDs) |
| `CallResult<T, E>` | Result type for RPC calls |
| `RoamError` | Standard error type for handlers |
| `Frame` | Wire frame for zero-copy access |

### roam-schema

| Type | Purpose |
|------|---------|
| `ServiceDetail` | Complete service metadata |
| `MethodDetail` | Single method metadata |
| `ArgDetail` | Single argument metadata |
| `ShapeKind` | Classified type information |
| `classify_shape()` | Convert Shape → ShapeKind |
| `is_tx()` / `is_rx()` | Check for streaming types |

### facet-core

| Type | Purpose |
|------|---------|
| `Shape` | Complete type introspection |
| `<T as Facet>::SHAPE` | Access shape for any `Facet` type |
| `shape.fully_qualified_type_path()` | Full module path for types |

## Tx/Rx Facet Implementation

This section documents experiments run in `rust/facet-experiments/` to figure out how to implement `Tx<T>` and `Rx<T>` so they work with `Connection::call_raw`.

### Requirements

For `Connection` to work its magic, `Tx<T>` and `Rx<T>` must:
- Have a **pokeable `stream_id: u64` field** - so Connection can walk args and set stream IDs
- **Expose `T` as a type parameter** - so codegen knows the element type
- **Serialize as just `u64`** - via `#[facet(proxy = u64)]`
- Have **opaque fields** for sender/receiver channels - they don't implement Facet

### Experiment Results (rust/facet-experiments/)

**Experiment 1: Fully opaque struct (`#[facet(opaque)]` on struct)**

```rust
#[derive(Facet)]
#[facet(opaque)]
pub struct FullyOpaque<T: 'static> {
    pub stream_id: u64,
    sender: mpsc::Sender<Vec<u8>>,
    _marker: PhantomData<fn(T)>,
}
```

Results:
- `type_params: []` — T is NOT exposed
- `ty: User(Opaque)` — not a struct
- Cannot convert to `PokeStruct` — no field access at all
- ❌ **Not usable** for Tx/Rx

**Experiment 2: Partial opaque (`#[facet(opaque)]` only on non-Facet fields)**

```rust
#[derive(Facet)]
pub struct PartiallyOpaque<T: 'static> {
    pub stream_id: u64,
    #[facet(opaque)]
    sender: mpsc::Sender<Vec<u8>>,
    #[facet(opaque)]
    _marker: PhantomData<fn(T)>,
}
```

Results:
- `type_params: ["T"]` — T IS exposed!
- Has 3 fields visible in shape
- CAN convert to `PokeStruct`
- CAN poke `stream_id` field and set it (verified: 0 → 42)
- Opaque fields accessible as `Poke` but inner structure hidden
- ✅ **This is what we need**

**Experiment 3: Container-level proxy for serialization**

Tx/Rx should serialize as just a `u64` (the stream ID), not the full struct. We use `#[facet(proxy = TxProxy)]` at the container level.

```rust
/// The proxy type - a transparent newtype over u64
#[derive(Facet, Clone, PartialEq)]
#[facet(transparent)]
pub struct TxProxy(pub u64);

/// Tx stream handle - serializes as just a u64 via TxProxy
#[derive(Facet)]
#[facet(proxy = TxProxy)]
pub struct Tx<T: 'static> {
    pub stream_id: u64,
    #[facet(opaque)]
    sender: mpsc::Sender<Vec<u8>>,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

// For SERIALIZATION: &Tx<T> -> TxProxy
impl<T: 'static> TryFrom<&Tx<T>> for TxProxy {
    type Error = std::convert::Infallible;
    fn try_from(tx: &Tx<T>) -> Result<Self, Self::Error> {
        Ok(TxProxy(tx.stream_id))
    }
}

// For DESERIALIZATION: TxProxy -> Tx<T>
// Creates a "hollow" Tx - the real sender gets injected by Connection after deserialization
impl<T: 'static> TryFrom<TxProxy> for Tx<T> {
    type Error = std::convert::Infallible;
    fn try_from(proxy: TxProxy) -> Result<Self, Self::Error> {
        let (sender, _rx) = mpsc::channel(1); // placeholder
        Ok(Tx {
            stream_id: proxy.0,
            sender,
            _marker: PhantomData,
        })
    }
}
```

Results:
```
=== Tx<i32> ===
type_identifier: Tx
module_path: Some("facet_experiments")
type_params: ["T"]
proxy: TxProxy (container-level)
fields (3):
  - stream_id: u64 (shape: u64)
  - sender: Opaque (shape: Opaque)
  - _marker: Opaque (shape: Opaque)

### Testing Poke on Tx ###
Initial stream_id: 0
✅ Successfully poked stream_id = 42
Final stream_id: 42

### Testing JSON Roundtrip ###
Original stream_id: 42
Serialized JSON: 42
Deserialized stream_id: 42
✅ JSON roundtrip successful!

### Testing Postcard Roundtrip ###
Original stream_id: 42
Serialized bytes: [42] (1 bytes)
Deserialized stream_id: 42
✅ Postcard roundtrip successful!

### Testing Tx embedded in a struct ###
Original: request_id=100, data_stream.stream_id=42
JSON: {"request_id":100,"data_stream":42}
✅ JSON roundtrip: request_id=100, data_stream.stream_id=42
Postcard bytes: [100, 42] (2 bytes)
✅ Postcard roundtrip: request_id=100, data_stream.stream_id=42
```

Key findings:
- ✅ **Container-level proxy works** - `Tx<i32>` shows `proxy: TxProxy (container-level)`
- ✅ **Can still poke `stream_id`** - Connection can walk args and set stream IDs
- ✅ **JSON serializes as bare `42`** - not the full struct
- ✅ **Postcard serializes as `[42]`** - single varint byte
- ✅ **Works when embedded** - `{"request_id":100,"data_stream":42}`
- ✅ **Type params preserved** - `["T"]` exposed for codegen introspection

### Direct `proxy = u64` is simplest

Tested `#[facet(proxy = u64)]` directly (no newtype needed):

```rust
#[derive(Facet)]
#[facet(proxy = u64)]
pub struct Tx<T: 'static> {
    pub stream_id: u64,
    #[facet(opaque)]
    sender: mpsc::Sender<Vec<u8>>,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

// For SERIALIZATION: &Tx<T> -> u64
impl<T: 'static> TryFrom<&Tx<T>> for u64 {
    type Error = std::convert::Infallible;
    fn try_from(tx: &Tx<T>) -> Result<Self, Self::Error> {
        Ok(tx.stream_id)
    }
}

// For DESERIALIZATION: u64 -> Tx<T>
impl<T: 'static> TryFrom<u64> for Tx<T> {
    type Error = std::convert::Infallible;
    fn try_from(stream_id: u64) -> Result<Self, Self::Error> {
        let (sender, _rx) = mpsc::channel(1); // placeholder
        Ok(Tx {
            stream_id,
            sender,
            _marker: PhantomData,
        })
    }
}
```

Results:
```
=== Tx2<i32> (direct u64 proxy) ===
type_identifier: Tx2
module_path: Some("facet_experiments")
type_params: ["T"]
proxy: u64 (container-level)
fields (3):
  - stream_id: u64 (shape: u64)
  - sender: Opaque (shape: Opaque)
  - _marker: Opaque (shape: Opaque)

### Testing Poke on Tx2 (direct u64 proxy) ###
Initial stream_id: 0
✅ Successfully poked stream_id = 42
Final stream_id: 42

### Testing JSON Roundtrip (Tx2 with direct u64 proxy) ###
Original stream_id: 42
Serialized JSON: 42
Deserialized stream_id: 42
✅ JSON roundtrip successful!
```

**Conclusion**: `#[facet(proxy = u64)]` is simpler than the newtype approach. No need for `TxProxy`/`RxProxy` types. Both approaches work identically.

### Final Recommended Tx/Rx Implementation

```rust
/// Tx<T> - caller sends to callee
#[derive(Facet)]
#[facet(proxy = u64)]
pub struct Tx<T: 'static> {
    pub stream_id: u64,
    #[facet(opaque)]
    sender: OutgoingSender,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

/// Rx<T> - caller receives from callee  
#[derive(Facet)]
#[facet(proxy = u64)]
pub struct Rx<T: 'static> {
    pub stream_id: u64,
    #[facet(opaque)]
    receiver: mpsc::Receiver<Vec<u8>>,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

// TryFrom<&Tx<T>> for u64 - serialization
// TryFrom<u64> for Tx<T> - deserialization (creates placeholder sender)
// Same pattern for Rx<T>
```

This gives us:
- ✅ `type_params: ["T"]` exposed for codegen
- ✅ `stream_id` pokeable (Connection can set it)
- ✅ Serializes as bare `u64` (JSON: `42`, Postcard: `[42]`)
- ✅ Opaque fields don't need Facet impl
- ✅ Works when embedded in structs

## Open Questions

1. **Borrowed returns**: How to handle `CallResult` with borrowed data? Current proc-macro uses `OwnedMessage` wrapper.

2. **Cancellation**: How should cancellation propagate through streams? Need `StreamClose` vs `StreamAbort` distinction?

3. **Backpressure**: Flow control via credits — how much should be exposed in generated API?

4. **Multi-service**: `RoutedDispatcher` pattern — should codegen support generating combined dispatchers?