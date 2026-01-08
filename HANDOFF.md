# Handoff: Connection Type and Tx/Rx Facet Implementation

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

**Current state** (what we're replacing): Manual message construction, serialization, request ID tracking - see `rust/codegen-test-consumer/src/lib.rs` for the painful manual approach.

## Completed Work

### Tx/Rx Facet Implementation ✅

Experiments in `rust/facet-experiments/` validated the approach. Updated `roam-session/src/lib.rs`:

**Before** (manual unsafe impl):
```rust
pub struct Tx<T: Facet<'static>> {
    stream_id: StreamId,  // private
    sender: OutgoingSender,
    _marker: PhantomData<fn(T)>,
}

#[allow(unsafe_code)]
unsafe impl<T: Facet<'static>> Facet<'static> for Tx<T> {
    const SHAPE: &'static Shape = &const { /* manual shape building */ };
}
```

**After** (derive with proxy):
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

impl<T: 'static> TryFrom<&Tx<T>> for u64 { /* serialization */ }
impl<T: 'static> TryFrom<u64> for Tx<T> { /* deserialization */ }
```

Key properties:
- `stream_id` is pokeable (Connection can walk args and set stream IDs)
- Serializes as just a `u64` on the wire (JSON: `42`, Postcard: `[42]`)
- Type parameter `T` exposed for codegen introspection (`type_params: ["T"]`)
- Opaque fields don't need Facet impl

### ConnectionHandle ✅

Added to `roam-session/src/lib.rs`:

- `ConnectionHandle` - cloneable handle for making outgoing calls
- `call_raw(method_id, payload)` - make raw RPC call
- `new_tx<T>()` / `new_rx<T>()` - allocate streams
- `HandleCommand` - communication with driver
- `CallError` - error type

### All Tests Pass ✅

```bash
cargo test --workspace  # All pass
```

## What's Next

### 1. Connection with Poke-based call_raw

Currently `call_raw` takes pre-serialized `payload: Vec<u8>`. Need:

```rust
// Takes a Poke so it can walk args, find Tx/Rx, bind stream IDs, then serialize
pub async fn call<T: Facet<'static>>(&self, method_id: u64, args: &mut T) -> Result<Vec<u8>, CallError>
```

The flow:
1. Create `Poke` from args
2. Walk structure, find `Tx`/`Rx` fields (by module_path + type_identifier, or facet's upcoming decl_id)
3. Allocate stream IDs and poke them into the `stream_id` fields
4. Serialize via facet (Tx/Rx become u64 via proxy)
5. Send request

### 2. Update Generated Client Code

Change codegen to use Connection instead of manual message building:

```rust
// Generated code should look like:
impl<C: Connection> CalculatorClient<C> {
    pub async fn add(&self, a: i32, b: i32) -> Result<i32, CallError> {
        let mut args = (a, b);
        let response = self.conn.call(method_id::ADD, &mut args).await?;
        facet_postcard::from_slice(&response).map_err(CallError::Decode)
    }
}
```

### 3. Update Tests

Replace manual message construction in tests with the new API.

## Key Files

| File | What |
|------|------|
| `rust/roam-session/src/lib.rs` | Tx/Rx with derive, ConnectionHandle |
| `rust/facet-experiments/src/main.rs` | Experiments validating the approach |
| `CODEGEN.md` | Full architecture docs and experiment results |
| `rust/roam-stream/src/driver.rs` | Uses ConnectionHandle |

## Experiment Results Summary

See `CODEGEN.md` section "Tx/Rx Facet Implementation" for full details.

**Key finding**: `#[facet(proxy = u64)]` at container level + `#[facet(opaque)]` on non-Facet fields gives us:
- Pokeable `stream_id` field
- Type params exposed
- Serializes as bare u64
- Works when embedded in structs

## Commands

```bash
# Build
cargo build --workspace

# Test
cargo test --workspace

# Run experiments
cargo run -p facet-experiments

# Check specific crate
cargo check -p roam-session
```

## Branch

`tx-rx-rename` - pushed to origin