# CALL_ON_ME: Layer Responsibilities (framed, stream, shm)

## Goal

Define all runtime layers and assign ownership clearly:

- where reconnection happens
- where request retry happens
- how channel binding works
- what is transport-specific vs transport-agnostic

This is architecture-only; no migration plan.

## Layer map

### Layer 0: Generated service clients (API layer)

Responsibility:
- Expose typed methods (`echo(message: String) -> Future<Result<...>>`).
- Hold method descriptor and typed args.
- Do not own transport behavior.
- Do not own reconnect/retry policy.
- Do not own channel binding logic.

Contract:
- Call into one session abstraction (`RpcSession`) and runtime typed helper.

### Layer 1: Session call runtime (transport-agnostic core)

Responsibility:
- Encode typed args with descriptor plans.
- Build request payload + request channel list.
- Execute request via session primitive (`call_raw`).
- Decode response payload (`ok_plan` / `err_plan`).
- Bind response `Rx<T>` channels into decoded result.

Owns:
- typed <-> raw boundary
- decode outcome mapping (`Ok`, user error, system error, protocol error)
- response channel binding semantics

Does not own:
- socket/websocket/shm I/O details
- reconnect decisions

### Layer 2: RpcSession abstraction (transport-agnostic interface)

Responsibility:
- One call primitive: send request and return `ResponseData`.
- Guarantee request/response message ordering and correlation by request id.
- Surface transport failures in a uniform error type.

Minimal shape sketch:

```rust
pub trait RpcSession: Send + Sync + 'static {
    fn call_raw(
        &self,
        descriptor: &'static MethodDescriptor,
        payload: Payload,
        metadata: Metadata,
    ) -> Pin<Box<dyn Future<Output = Result<ResponseData, TransportError>> + Send + 'static>>;

    unsafe fn bind_response_channels_by_plan(
        &self,
        response_ptr: *mut (),
        response_plan: &'static RpcPlan,
        channels: &[ChannelId],
    );
}
```

Note:
- binding hook may be kept here or folded into Layer 1 helper; either is fine as long as ownership is singular.

### Layer 3: Session implementations (transport-facing adapters)

Implementors:
- `ConnectionHandle` (already connected)
- reconnecting message client (`FramedClient`)
- reconnecting byte-stream client (`roam_stream::Client`)
- shm session handle (no network reconnect)

Responsibility:
- translate `call_raw` into concrete request/response message exchange
- manage current live handle
- reconnect when policy allows
- clear/refresh stale handle on disconnection

### Layer 4: Transport connectors and framing

Responsibility:
- establish underlying connection:
  - framed message transport (WebSocket/native message transport)
  - byte stream transport (TCP/Unix + length framing)
  - shm transport (shared memory setup + doorbells)
- handshake and protocol version negotiation

Does not own:
- typed decode/bind behavior
- per-method descriptor logic

## Cross-cutting ownership

### Reconnection ownership

Owner: Layer 3 session implementations (`FramedClient`, `Client`).

Rules:
- reconnect only on transport-level failures (`ConnectionClosed`, `DriverGone`, connect errors)
- do not reconnect on protocol/user errors (`UnknownMethod`, payload decode errors, user error)
- backoff and attempt limits come from `RetryPolicy`

SHM:
- generally no reconnect loop like network transports; failure is terminal unless explicit reattach policy is added.

### Request retry ownership

Owner: Layer 3 session implementations.

Definition:
- retry means re-issuing the same logical call after reconnect.

Rules:
- retry only when request outcome is unknown because transport dropped
- do not retry encode failures
- do not retry completed RPC responses (even if they contain app/system error payloads)

Important semantic note:
- retries are at-least-once from server perspective unless protocol provides idempotency tokens.

### Channel binding ownership

Split ownership:

1. Request-side channel extraction
- Owner: Layer 1 runtime helper
- Action: walk args by plan, collect channel ids in declaration order, put into request framing

2. Server-side arg channel patch/bind
- Owner: dispatch runtime (`dispatch_call`)
- Action: patch request channel ids into deserialized args, then bind Tx/Rx endpoints

3. Response-side channel binding
- Owner: Layer 1 runtime helper (or delegated to `RpcSession` binding hook)
- Action: after decoding `Ok` value, walk decoded result by plan and bind `Rx<T>` from response channel list

Invariant:
- channel binding behavior must be identical regardless of transport (framed, stream, shm).

## Transport-specific notes

### Framed (message transports)

- Reconnect/retry happens in reconnecting framed session impl.
- `call_raw` works with message transport connector.
- Response channel binding must not be a no-op.

### Stream (byte stream TCP/Unix)

- Uses length-prefixed framing internally.
- Reconnect/retry happens in stream `Client`.
- Maintains current underlying handle for response channel binding.

### SHM

- No network connector loop by default.
- Session still uses same call/decode/bind semantics.
- Any reconnect policy should be explicit, not implicit.

## Middleware/tracing placement

Preferred placement:
- middleware at dispatch/request context boundaries.

If client-side decoration is needed:
- use internal `RpcSession` decorator, not public generic `Caller` surface.

## Naming note (`roam-stream`)

`roam-stream` should mean byte-stream transport support.

If it re-exports framed APIs for compatibility, that is a surface/packaging choice; ownership still belongs to the framed/session layers above.
