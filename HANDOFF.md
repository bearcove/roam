# Handoff: Connection Type and Tx/Rx Facet Implementation

## Status: Streaming Dispatch Implemented (codegen approach) ✅

The core streaming RPC infrastructure works end-to-end using codegen-time type knowledge.

**Next step**: Implement the Poke-based `call` method for cleaner client DX.

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
- Opaque fields can still be accessed/replaced via Poke - just can't see inside them

### 2. Streaming Dispatch Code Generation ✅

Server-side streaming dispatch works via generated code in `roam-codegen/src/targets/rust.rs`.

### 3. End-to-End Streaming Tests ✅

Tests in `codegen-test-consumer/src/lib.rs` verify full streaming flow.

---

## Next: Poke-based Client `call` with `roam::channel`

### Desired DX

```rust
// sum(numbers: Rx<i32>) -> i64
// Schema says Rx<i32> = server receives i32s from caller

let (tx, rx) = roam::channel::<i32>();

let fut = client.sum(rx);  // pass rx to the call, keep tx

// Use tx to send numbers to the server
tx.send(1).await;
tx.send(2).await;
tx.send(3).await;
drop(tx);  // signals end of stream

let sum = fut.await?;  // returns 6
```

### Key Insight: Channel Semantics (like regular mpsc)

This matches standard channel conventions - just like `tokio::sync::mpsc`:
- `(tx, rx) = channel()` → tx sends, rx receives
- If caller wants to **send** data: pass `rx`, keep `tx`
- If caller wants to **receive** data: pass `tx`, keep `rx`

When schema says `Rx<T>` (caller sends to callee):
- User creates `(tx, rx) = roam::channel::<T>()`
- User **keeps** `tx` (the sender) to send data
- User **passes** `rx` (the receiver) to the call
- Connection takes the `mpsc::Receiver` from `rx`, registers it as outgoing stream

When schema says `Tx<T>` (callee sends to caller):
- User creates `(tx, rx) = roam::channel::<T>()`
- User **keeps** `rx` (the receiver) to receive data
- User **passes** `tx` (the sender) to the call
- Connection takes the sender, callee will use it to send back data

### Internal Channel Type

The mpsc channel is `Sender<Vec<u8>>` / `Receiver<Vec<u8>>`:

```rust
pub fn channel<T>() -> (Tx<T>, Rx<T>) {
    let (sender, receiver) = mpsc::channel::<Vec<u8>>(64);
    (
        Tx { stream_id: 0, sender: Some(sender), _marker: PhantomData },
        Rx { stream_id: 0, receiver: Some(receiver), _marker: PhantomData },
    )
}
```

The `T` type parameter is only used at the public API boundary for ser/deser:

```rust
impl<T: Facet<'static>> Tx<T> {
    pub async fn send(&self, value: T) -> Result<(), TxError> {
        let bytes = facet_postcard::to_vec(&value)?;
        self.sender.as_ref().ok_or(TxError::Taken)?.send(bytes).await?;
        Ok(())
    }
}

impl<T: Facet<'static>> Rx<T> {
    pub async fn recv(&mut self) -> Result<Option<T>, RxError> {
        match self.receiver.as_mut().ok_or(RxError::Taken)?.recv().await {
            Some(bytes) => Ok(Some(facet_postcard::from_slice(&bytes)?)),
            None => Ok(None),
        }
    }
}
```

The connection driver only deals with `Vec<u8>` - doesn't need to know `T`.

### Poke-based `call` Implementation

```rust
impl ConnectionHandle {
    pub async fn call<T: Facet<'static>>(
        &self,
        method_id: u64,
        args: &mut T,
    ) -> Result<Vec<u8>, CallError> {
        // Walk args with Poke, find Rx fields, bind them
        let poke = Poke::new(args);
        self.walk_and_bind_streams(poke)?;
        
        // Serialize (Rx becomes just stream_id via proxy)
        let payload = facet_postcard::to_vec(args).map_err(CallError::Encode)?;
        
        self.call_raw(method_id, payload).await
    }
    
    fn walk_and_bind_streams(&self, poke: Poke<'_, '_>) -> Result<(), BindError> {
        let shape = poke.shape();
        
        if is_rx_shape(shape) {
            let mut ps = poke.into_struct()?;
            
            // Allocate stream_id
            let stream_id = self.allocate_stream_id();
            
            // Poke stream_id into the Rx
            ps.field_by_name("stream_id")?.set(stream_id)?;
            
            // Take the mpsc::Receiver out of the Rx
            let mut receiver_poke = ps.field_by_name("receiver")?;
            let receiver: Option<mpsc::Receiver<Vec<u8>>> = receiver_poke.get_mut()?.take();
            
            // Register with stream registry (connection will drain and send over wire)
            if let Some(recv) = receiver {
                self.register_outgoing_stream(stream_id, recv);
            }
            
            return Ok(());
        }
        
        // Recurse into struct fields, enums, tuples, etc.
        if let Ok(mut ps) = poke.into_struct() {
            for i in 0..ps.field_count() {
                self.walk_and_bind_streams(ps.field(i)?)?;
            }
        }
        // ... handle other types
        
        Ok(())
    }
}
```

### Changes Needed

1. **`roam::channel<T>()` function** - creates unbound `(Tx<T>, Rx<T>)` pair with `stream_id: 0`

2. **Make sender/receiver fields `Option`** - so we can `.take()` them:
   ```rust
   pub struct Tx<T: 'static> {
       pub stream_id: StreamId,
       #[facet(opaque)]
       sender: Option<mpsc::Sender<Vec<u8>>>,
       #[facet(opaque)]
       _marker: PhantomData<T>,
   }
   ```

3. **Implement Poke-based walk** in `ConnectionHandle::call`

4. **Update driver** to handle the taken receivers (drain mpsc, send as Data messages)

### TODO: `streaming.data.invalid`

Since serialization/deserialization happens in `Tx::send` / `Rx::recv` (not in the connection driver), we need a mechanism to report invalid data errors back through the protocol. This is deferred for now.

---

## Key Files

| File | What |
|------|------|
| `rust/roam-session/src/lib.rs` | Tx/Rx with derive, ConnectionHandle, StreamRegistry |
| `rust/roam-stream/src/driver.rs` | Driver that handles I/O loop |
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