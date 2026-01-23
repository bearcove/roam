# roam-next: Minimize Monomorphization in Dispatch

## Goal

Reduce code bloat by making dispatch helpers non-generic. Currently, `dispatch_call<A, R, E, F, Fut>()`
gets monomorphized for every RPC method. With 50 methods, that's 50 copies of the same deserialization,
middleware, and serialization logic.

**New approach:** Generated code knows types and calls non-generic helpers that work via Shape + pointer.

## Current State

```
rust/roam-next/           # Prototype with prepare() API
rust/roam-session/        # Production code with generic dispatch_call()
```

The prototype proves the concept works. Now we integrate it into roam-session.

## Design

### Non-Generic Helpers

```rust
// Deserialize + run middleware (ONE copy, not N)
pub async unsafe fn prepare(
    args_ptr: *mut (),
    args_shape: &'static Shape,
    payload: &[u8],
    ctx: &mut Context,
    middleware: &[Arc<dyn Middleware>],
) -> Result<(), DispatchError>;

// Serialize and send OK response (ONE copy)
pub async fn send_ok_response(
    result_ptr: *const (),
    result_shape: &'static Shape,
    driver_tx: &Sender<DriverMessage>,
    conn_id: ConnectionId,
    request_id: u64,
    channels: Vec<u64>,
);

// Serialize and send error response (ONE copy)
pub async fn send_error_response(
    error_ptr: *const (),
    error_shape: &'static Shape,
    driver_tx: &Sender<DriverMessage>,
    conn_id: ConnectionId,
    request_id: u64,
);
```

### Middleware Trait (Peek-based)

```rust
pub trait Middleware: Send + Sync {
    fn intercept<'a>(
        &'a self,
        ctx: &'a mut Context,
        args: Peek<'_, 'static>,  // Can inspect args via reflection!
    ) -> Pin<Box<dyn Future<Output = Result<(), Rejection>> + Send + 'a>>;
}
```

### Generated Dispatcher

```rust
pub struct TestbedDispatcher<H> {
    handler: H,
    middleware: Vec<Arc<dyn Middleware>>,
}

impl<H> TestbedDispatcher<H> {
    pub fn new(handler: H) -> Self {
        Self { handler, middleware: Vec::new() }
    }

    pub fn with_middleware<M: Middleware + 'static>(mut self, mw: M) -> Self {
        self.middleware.push(Arc::new(mw));
        self
    }
}
```

### Generated dispatch_* Methods

```rust
fn dispatch_echo(&self, cx: Context, payload: Vec<u8>, registry: &mut ChannelRegistry)
    -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>
{
    let handler = self.handler.clone();
    let middleware = self.middleware.clone();
    let driver_tx = registry.driver_tx();
    let conn_id = cx.conn_id;
    let request_id = cx.request_id.raw();

    Box::pin(async move {
        // 1. Allocate args on stack
        let mut args = MaybeUninit::<(String,)>::uninit();

        // 2. Prepare: deserialize + middleware (NON-GENERIC)
        unsafe {
            if let Err(e) = prepare(
                args.as_mut_ptr().cast(),
                <(String,)>::SHAPE,
                &payload,
                &mut cx,
                &middleware,
            ).await {
                send_dispatch_error(e, driver_tx, conn_id, request_id).await;
                return;
            }
        }

        // 3. Read args (monomorphized but tiny)
        let (message,) = unsafe { args.assume_init_read() };

        // 4. Call handler (monomorphized - unavoidable)
        let result = handler.echo(&cx, message).await;

        // 5. Send response (NON-GENERIC)
        match result {
            Ok(value) => {
                send_ok_response(
                    &value as *const _ as *const (),
                    <String>::SHAPE,
                    &driver_tx,
                    conn_id,
                    request_id,
                    vec![], // channels from result
                ).await;
            }
            Err(error) => {
                send_error_response(
                    &error as *const _ as *const (),
                    <EchoError>::SHAPE,
                    &driver_tx,
                    conn_id,
                    request_id,
                ).await;
            }
        }
    })
}
```

### User Ergonomics

```rust
// Define handler
struct MyHandler;
impl Testbed for MyHandler {
    async fn echo(&self, cx: &Context, message: String) -> String {
        message
    }
}

// Create dispatcher with middleware
let dispatcher = TestbedDispatcher::new(MyHandler)
    .with_middleware(AuthMiddleware::new())
    .with_middleware(RateLimiter::new());

// Use as before
let client = connect(connector, config, dispatcher);
```

## Implementation Plan

### Phase 1: Update roam-session dispatch infrastructure

- [x] **1.1** Update `Middleware` trait to take `args: SendPeek` (Peek wrapper with unsafe Send/Sync)
- [x] **1.2** Add `prepare()` function (non-generic deserialize + middleware)
- [ ] **1.3** Add `send_ok_response()` function (non-generic serialize + send)
- [ ] **1.4** Add `send_error_response()` function (non-generic serialize + send)
- [ ] **1.5** Keep old `dispatch_call` temporarily for compatibility

### Phase 2: Update macro codegen (roam-macros)

- [ ] **2.1** Add `middleware: Vec<Arc<dyn Middleware>>` field to generated dispatcher
- [ ] **2.2** Add `with_middleware()` builder method
- [ ] **2.3** Update generated `dispatch_*` methods to use new pattern:
  - Allocate `MaybeUninit` for args
  - Call `prepare()` with Shape
  - Read args and call handler
  - Call `send_*_response()` with Shape
- [ ] **2.4** Handle channel ID patching via Poke (non-generic)
- [ ] **2.5** Handle stream binding via Poke (non-generic)

### Phase 3: Cleanup

- [ ] **3.1** Remove old generic `dispatch_call` / `dispatch_call_infallible`
- [ ] **3.2** Remove `WithMiddleware` wrapper (superseded)
- [ ] **3.3** Delete or merge `roam-next` crate (concepts moved to roam-session)
- [ ] **3.4** Update any tests

### Phase 4: Future work (not this PR)

- [ ] Client-side middleware (intercept outgoing calls)
- [ ] Middleware that can modify args (Poke, not just Peek)

## Open Questions

1. **Channel ID patching** - currently uses `patch_channel_ids()` which is generic.
   Can we do this via Poke (non-generic)? The roam-next prototype has a TODO for this.

2. **Stream binding** - currently `registry.bind_streams(&mut args)` uses reflection.
   Should work as-is since it already uses Facet reflection.

3. **Response channel collection** - `collect_channel_ids()` walks the result to find Tx/Rx.
   This already uses reflection, should be fine.

## Files to Modify

```
rust/roam-session/src/
  dispatch.rs      # Add prepare(), send_*_response(), update Middleware trait
  middleware.rs    # Update Middleware trait, remove WithMiddleware
  lib.rs           # Re-exports

rust/roam-macros/src/
  lib.rs           # Update codegen for new dispatch pattern

rust/roam-next/    # Eventually delete or merge
```

## Success Criteria

1. All existing tests pass
2. Middleware can peek at deserialized args
3. `cargo llvm-lines` shows reduced monomorphization
4. No regression in runtime performance
