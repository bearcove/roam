+++
title = "migrating from v6 to v7"
description = "Practical API migration notes for Rust users moving to roam v7"
weight = 16
+++

# Migration guide (Rust API)

This page maps the old v6 Rust API surface to the current v7 API.

## High-level changes

1. Service impls no longer receive `&Context`; they receive `call: impl roam::Call<T, E>`.
2. Generated server trait name changed from `{Service}` to `{Service}Server`.
3. Generated client calls now return `ResponseParts<T>` (with `ret` and `metadata`), not bare `T`.
4. `ConnectionHandle` is no longer a `Caller`; clients are built from `driver.caller()`.
5. Session setup moved from `accept_framed` / `initiate_framed` to `session::acceptor` / `session::initiator` builders.
6. Channels are now `Tx<T, N>` / `Rx<T, N>` with const-generic initial credit (`N`, default `16`).

## Service macro changes

v6 style:

```rust
#[roam::service]
trait Adder {
    async fn add(&self, a: i32, b: i32) -> i32;
}

impl Adder for MyAdder {
    async fn add(&self, _cx: &roam::Context, a: i32, b: i32) -> i32 {
        a + b
    }
}
```

v7 style:

```rust
#[roam::service]
trait Adder {
    async fn add(&self, a: i32, b: i32) -> i32;
}

impl AdderServer for MyAdder {
    async fn add(&self, call: impl roam::Call<i32, core::convert::Infallible>, a: i32, b: i32) {
        call.ok(a + b).await;
    }
}
```

In v7, handlers reply explicitly with `call.ok(...)`, `call.err(...)`, or `call.reply(...)`.

## Client return type changes

v6 call sites often expected bare values:

```rust
let sum: i32 = client.add(3, 5).await?;
```

In v7:

```rust
let response = client.add(3, 5).await?;
let sum = response.ret;
let _metadata = response.metadata;
```

`ResponseParts<T>` implements `Deref<Target = T>`, so many existing read-only usages continue to work.

## Session and driver setup

v6 used handshake helpers returning a handle/driver pair:

```rust
use roam_session::{accept_framed, initiate_framed, HandshakeConfig, NoDispatcher};
let (handle, _incoming, driver) = initiate_framed(transport, HandshakeConfig::default(), NoDispatcher).await?;
```

v7 setup is explicit:

```rust
let (mut session, handle, _session_handle) = roam_core::session::initiator(conduit)
    .establish()
    .await?;

let mut driver = roam_core::Driver::new(handle, dispatcher_or_handler, roam_types::Parity::Odd);
let caller = driver.caller();
```

Server side is analogous via `roam_core::session::acceptor(conduit).establish().await?`.

## Old-to-new symbol map

| v6 | v7 | Notes |
|---|---|---|
| `impl MyService for Handler` | `impl MyServiceServer for Handler` | Generated server trait renamed |
| `fn method(&self, cx: &Context, ...) -> T` | `fn method(&self, call: impl Call<T, E>, ...) -> Future<Output = ()>` | Explicit reply path |
| `MyServiceClient::new(connection_handle)` | `MyServiceClient::new(driver.caller())` | `ConnectionHandle` is no longer a `Caller` |
| `accept_framed` / `initiate_framed` | `session::acceptor` / `session::initiator` + `.establish()` | New session builders |
| `Tx<T>` / `Rx<T>` | `Tx<T, N>` / `Rx<T, N>` | `N` is initial credit (default `16`) |
| `client.method(...).with_metadata(...)` | no generated call-builder equivalent | Use lower-level request construction if needed |

## Migration checklist

1. Rename service impls to `{Service}Server`.
2. Replace `&Context` parameters with `call: impl roam::Call<Ok, Err>`.
3. Replace returned values with `call.ok(...)` / `call.err(...)`.
4. Update client call sites to read `.ret` (and `.metadata` when needed).
5. Update bootstrap code to `session::{initiator,acceptor}` and instantiate `Driver`.
6. Switch client construction from connection handles to `driver.caller()`.
7. If needed, annotate channel credit explicitly with `Tx<T, N>` / `Rx<T, N>`.
