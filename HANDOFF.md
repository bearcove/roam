# Handoff: Connection Type and Tx/Rx Facet Implementation

## Status: Streaming Dispatch Implemented ✅

The core streaming RPC infrastructure is now working end-to-end.

## Completed Work

### 1. Tx/Rx Facet Implementation ✅

Updated `roam-session/src/lib.rs` with derive-based Tx/Rx:

```rust
#[derive(Facet)]
#[facet(proxy = u64)]
pub struct Tx<T: 'static> {
    pub stream_id: StreamId,  // public, pokeable!
    #[facet(opaque)]
    sender: OutgoingSender,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}
```

Key properties:
- `stream_id` is pokeable (Connection can walk args and set stream IDs)
- Serializes as just a `u64` on the wire (JSON: `42`, Postcard: `[42]`)
- Type parameter `T` exposed for codegen introspection
- Opaque fields don't need Facet impl

### 2. ConnectionHandle with `call` method ✅

Added to `roam-session/src/lib.rs`:

```rust
impl ConnectionHandle {
    /// Make a typed RPC call with automatic serialization.
    pub async fn call<T: Facet<'static>>(
        &self,
        method_id: u64,
        args: &T,
    ) -> Result<Vec<u8>, CallError> {
        let payload = facet_postcard::to_vec(args).map_err(CallError::Encode)?;
        self.call_raw(method_id, payload).await
    }
    
    /// Allocate a new outgoing stream (caller → callee).
    pub fn new_tx<T: 'static>(&self) -> Tx<T>
    
    /// Allocate a new incoming stream (callee → caller).
    pub fn new_rx<T: 'static>(&self) -> Rx<T>
}
```

### 3. Streaming Dispatch Code Generation ✅

Updated `roam-codegen/src/targets/rust.rs` to generate proper streaming dispatch:

- `dispatch_streaming` method in ServiceDispatcher impl
- Stream IDs decoded from payload (streams serialize as u64 via proxy)
- Tx/Rx created via registry methods with proper stream binding
- Type inversion: caller's Tx becomes handler's Rx (and vice versa)
- Handler cloned for 'static future (requires Clone bound)

Generated code looks like:
```rust
fn dispatch_streaming(&self, method_id: u64, payload: Vec<u8>, registry: &mut StreamRegistry) -> ... {
    match method_id {
        method_id::SUM_STREAM => {
            let (numbers) = facet_postcard::from_slice::<(u64)>(&payload)?;
            let numbers_rx = registry.register_incoming(numbers);
            let numbers = Rx::<i32>::new(numbers, numbers_rx);
            let handler = self.handler.clone();
            Box::pin(async move {
                match handler.sum_stream(numbers).await { ... }
            })
        }
        // ...
    }
}
```

### 4. Fixed Send Safety Bug ✅

Fixed `ConnectionHandle::route_data` to not hold MutexGuard across await:
- Added `prepare_route_data` that returns (Sender, payload) synchronously
- Actual send happens after releasing the lock

### 5. End-to-End Streaming Tests ✅

Two new tests in `codegen-test-consumer/src/lib.rs`:

1. `streaming_sum_stream_roundtrip` - Client pushes numbers via Tx, server sums them
2. `streaming_range_roundtrip` - Server pushes numbers via Tx (inverted Rx from client perspective)

Both tests verify the full streaming flow including:
- Stream ID allocation
- Data transmission
- Stream closure
- Response handling

## Usage Example

```rust
// Client side
let (handle, driver) = establish_initiator(io, hello, NoOpDispatcher).await?;
tokio::spawn(driver.run());

// Create stream and make call
let tx: Tx<i32> = handle.new_tx();
let stream_id = tx.stream_id();
let payload = facet_postcard::to_vec(&(stream_id,))?;

let response_fut = handle.call_raw(method_id::SUM_STREAM, payload);

// Send data on stream
tx.send(&1).await?;
tx.send(&2).await?;
drop(tx); // Close stream

let response = response_fut.await?;
let result: CallResult<i64, Never> = facet_postcard::from_slice(&response)?;
```

## What's Next

### 1. Generated Client Code

Update codegen to generate client implementations that hide the boilerplate:

```rust
// Goal:
let client = CalculatorClient::new(handle);
let result = client.add(2, 3).await?;  // Just works!

// For streaming:
let tx = client.new_numbers_stream();
let result_fut = client.sum_stream(tx);
tx.send(&1).await?;
drop(tx);
let sum = result_fut.await?;
```

### 2. Clean up test duplication

The NoOpDispatcher is duplicated in tests - could be extracted to a helper.

### 3. Error handling improvements

Currently errors are encoded as simple byte markers. Should properly serialize RoamError variants.

## Key Files

| File | What |
|------|------|
| `rust/roam-session/src/lib.rs` | Tx/Rx with derive, ConnectionHandle, StreamRegistry |
| `rust/roam-stream/src/driver.rs` | Driver that handles I/O loop, uses ConnectionHandle |
| `rust/roam-codegen/src/targets/rust.rs` | Rust client/server codegen with streaming support |
| `rust/codegen-test-consumer/src/lib.rs` | Tests including streaming roundtrips |

## Commands

```bash
# Build
cargo build --workspace

# Test
cargo test --workspace

# Test streaming specifically
cargo test -p codegen-test-consumer streaming

# Check specific crate
cargo check -p roam-session
```

## Branch

`tx-rx-rename` - pushed to origin