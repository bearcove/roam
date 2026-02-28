# Channels are easy and fun

## `sum`

```rust
#[roam::service]
trait Adder {
    async fn sum(&self, numbers: Rx<i32>) -> i64;
}
```

Generated handler trait:

```rust
trait AdderServer {
    async fn sum(&self, call: impl Call<i64, Infallible>, numbers: Rx<i32>);
}
```

Generated client method:

```rust
impl AdderClient {
    async fn sum(&self, numbers: Rx<i32>) -> Result<ResponseParts<i64>, RoamError>;
}
```

The handler receives `Rx<i32>`. It calls `numbers.recv()` in a loop, accumulating
values, then returns the total.

The caller creates a channel pair:

```rust
let (tx, rx) = roam::channel::<i32>();
```

The caller keeps `tx` to send items. It passes `rx` to the call:

```rust
client.sum(rx).await
```

`Rx<i32>` is in arg position. Per the spec, the caller allocates the channel ID.
The framework binds `rx` so the handler can receive from it. The framework also
needs to bind `tx` so the caller can send into the same channel.

## `start_ingester` — Rx in return position

```rust
#[roam::service]
trait Pipeline {
    async fn start_ingester(&self) -> Rx<Job>;
}
```

Generated handler trait:

```rust
trait PipelineServer {
    async fn start_ingester(&self, call: impl Call<Rx<Job>, Infallible>);
}
```

Generated client method:

```rust
impl PipelineClient {
    async fn start_ingester(&self) -> Result<ResponseParts<Rx<Job>>, RoamError>;
}
```

The handler starts a background processor, creates a channel pair, keeps
`tx` to pull work from, and returns `rx` via `call.ok(rx)`.

The caller receives `Rx<Job>` in the response. But `Rx` means the handler
receives — so the caller sends. The caller gets back a bound `Tx<Job>`
(extracted from the response's channel bindings) and uses it to feed jobs
to the handler.

`Rx<Job>` is in return position. Per the spec, the **callee** allocates the
channel ID. The direction is the same as in arg position: the caller sends,
the handler receives. Position only affects who allocates the ID.
