# Handoff: Poke-based `call` with `roam::channel`

## Status: Codegen Updated, Driver Work Remaining

**Completed**:
- `ReceiverSlot` and `SenderSlot` wrappers with `#[facet(opaque)]` for Poke reflection
- `roam::channel<T>()` function creates unbound `(Tx<T>, Rx<T>)` pairs
- `StreamRegistry` simplified - no longer stores outgoing receivers (driver will own them)
- Codegen fixed to use correct Tx/Rx semantics (NO INVERSION)
- All 11 spec tests passing

**Remaining**:
- Driver needs `FuturesUnordered` to poll outgoing receivers and send as Data messages
- Client-side Poke-based `call` implementation

## Key Insight: Tx/Rx Semantics (Critical)

The function signature is both what the client calls AND what the server implements.
**NO INVERSION** at any point:

- `Rx<T>` in schema = server **receives** from client (client sends)
- `Tx<T>` in schema = server **sends** to client (client receives)

```rust
// Schema: sum(numbers: Rx<i32>) -> i64
// Server receives i32s from client

// Client side:
let (tx, rx) = roam::channel::<i32>();
let fut = client.sum(rx);  // pass rx (receiver goes to server)
tx.send(1).await;          // keep tx to send values
drop(tx);
let sum = fut.await?;

// Server side:
async fn sum(&self, mut numbers: Rx<i32>) -> Result<i64, _> {
    while let Some(n) = numbers.recv().await? { ... }
}
```

## Completed Work

### 1. ReceiverSlot/SenderSlot Wrappers

In `roam-session/src/lib.rs`:

```rust
#[derive(Facet)]
#[facet(opaque)]
pub struct ReceiverSlot {
    inner: Option<mpsc::Receiver<Vec<u8>>>,
}

impl ReceiverSlot {
    pub fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self { Self { inner: Some(rx) } }
    pub fn empty() -> Self { Self { inner: None } }
    pub fn take(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> { self.inner.take() }
}
```

The `#[facet(opaque)]` allows Poke to access `&mut ReceiverSlot` directly.

### 2. Rx<T> and Tx<T> Updated

```rust
pub struct Rx<T: 'static> {
    pub stream_id: StreamId,
    pub receiver: ReceiverSlot,  // pokeable!
    _marker: PhantomData<T>,
}

pub struct Tx<T: 'static> {
    pub stream_id: StreamId,
    pub sender: SenderSlot,  // pokeable!
    _marker: PhantomData<T>,
}
```

### 3. `roam::channel<T>()` Function

```rust
pub fn channel<T: 'static>() -> (Tx<T>, Rx<T>) {
    let (sender, receiver) = mpsc::channel::<Vec<u8>>(64);
    (
        Tx::unbound(sender),
        Rx::unbound(receiver),
    )
}
```

Creates unbound pairs with `stream_id: 0`. The connection binds them when `call` is invoked.

### 4. StreamRegistry Simplified

Removed:
- `outgoing: HashMap<StreamId, mpsc::Receiver<Vec<u8>>>`
- `outgoing_notify: Arc<Notify>`
- `poll_outgoing()` method
- `OutgoingPoll` enum

The driver will now own outgoing receivers in `FuturesUnordered` instead of routing through the registry.

### 5. Codegen Updated

In `roam-codegen/src/targets/rust.rs`:

Server dispatch creates channels and registers with registry:
```rust
// For Rx<T> args (server receives from client):
let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
registry.register_incoming(stream_id, tx);  // route incoming Data to tx
let arg = Rx::<T>::new(stream_id, rx);      // handler uses rx

// For Tx<T> args (server sends to client):
let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
registry.register_outgoing_credit(stream_id);  // credit tracking only
let arg = Tx::<T>::new(stream_id, tx);         // handler uses tx
// rx goes to driver for sending as Data messages
```

## Next Steps

### 1. Driver FuturesUnordered for Outgoing

The driver needs to poll `mpsc::Receiver<Vec<u8>>` channels and send Data messages:

```rust
// In driver.rs
struct OutgoingStream {
    stream_id: StreamId,
    receiver: mpsc::Receiver<Vec<u8>>,
}

// Add to Driver state:
outgoing_streams: FuturesUnordered<OutgoingStreamFuture>,

// In run loop:
select! {
    // ... existing arms ...
    
    Some(data) = outgoing_streams.next() => {
        // Send Data message for this stream
        self.send(Message::Data { stream_id, payload: data }).await?;
    }
}
```

Need to figure out:
- How codegen passes the `rx` from `Tx<T>` args to the driver
- Credit flow for outgoing streams
- End-of-stream signaling

### 2. Client-side Poke-based `call`

```rust
impl ConnectionHandle {
    pub async fn call<T: Facet<'static>>(
        &self,
        method_id: u64,
        args: &mut T,
    ) -> Result<Vec<u8>, CallError> {
        // Walk args with Poke, find Rx fields, take receivers, bind stream IDs
        self.walk_and_bind_streams(Poke::new(args))?;
        
        let payload = facet_postcard::to_vec(args)?;
        self.call_raw(method_id, payload).await
    }
    
    fn walk_and_bind_streams(&self, poke: Poke<'_, '_>) -> Result<(), BindError> {
        if is_rx_shape(poke.shape()) {
            let mut ps = poke.into_struct()?;
            let stream_id = self.allocate_stream_id();
            ps.field_by_name("stream_id")?.set(stream_id)?;
            
            let slot = ps.field_by_name("receiver")?.get_mut::<ReceiverSlot>()?;
            if let Some(recv) = slot.take() {
                // Pass to driver somehow
                self.register_outgoing_stream(stream_id, recv);
            }
        }
        // Recurse into struct fields...
    }
}
```

## Key Files

| File | What |
|------|------|
| `rust/roam-session/src/lib.rs` | Tx/Rx, ReceiverSlot/SenderSlot, StreamRegistry |
| `rust/roam-stream/src/driver.rs` | Driver I/O loop - needs FuturesUnordered |
| `rust/roam-codegen/src/targets/rust.rs` | Rust codegen with streaming dispatch |
| `rust/roam/src/lib.rs` | Re-exports including `channel()` |

## Commands

```bash
# Build and test
cargo build --workspace
cargo nextest run --workspace

# Spec tests (cross-language)
cargo nextest run -p spec-tests

# Check specific crate
cargo check -p roam-session
```
