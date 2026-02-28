+++
title = "RPC"
description = "Services, method identity, handlers, and callers"
weight = 12
+++

# RPC concepts

> r[rpc]
>
> The RPC layer sits on top of connections. It defines how requests are made,
> how responses are returned, and how data flows over channels.

> r[rpc.service]
>
> A service is a set of methods. In Rust, a service is defined as a trait
> annotated with `#[roam::service]`. Methods only take `&self` — a service
> does not carry mutable state. Any state must be managed externally
> (e.g. behind an `Arc<Mutex<_>>` or similar).

> r[rpc.service.methods]
>
> Each method in a service is an async function. Its arguments and return
> type must implement `Facet`. The `#[roam::service]` macro adds a `&Context`
> parameter in first position when generating the handler trait.

> r[rpc.method-id]
>
> Every method has a unique 64-bit identifier derived from its service name,
> method name, and signature. This is what gets sent on the wire in `Request`
> messages.

> r[rpc.method-id.no-collisions]
>
> The method ID ensures that different services can have methods with the same
> name without collision, and that changing a method's signature produces a
> different ID (making the change visibly incompatible rather than silently
> wrong).

> r[rpc.method-id.algorithm]
>
> The exact algorithm for computing method IDs is defined in the
> [signature specification](../sig/). Other language implementations
> receive pre-computed method IDs from code generation.

> r[rpc.schema-evolution]
>
> Adding new methods to a service is always safe — peers that don't know about
> a method simply report it as unknown.
>
> Most other changes are breaking:
>
>   * Renaming a service or method
>   * Changing argument types, order, or return type
>   * Changing the structure of any type used in the signature (field names,
>     order, enum variants)
>   * Substituting container types (e.g. `Vec<T>` → `HashSet<T>`)
>
> Argument *names* are not part of the wire format and can be changed freely.
> Only types and their order matter.

> r[rpc.one-service-per-connection]
>
> Each connection is bound to exactly one service. If a peer needs to talk
> multiple protocols, it opens additional virtual connections — one per service.

> r[rpc.handler]
>
> A handler handles incoming requests on a connection. It is a user-provided
> implementation of a service trait. The roam runtime takes care of
> deserializing arguments, routing to the right method, and sending back responses.

> r[rpc.caller]
>
> A caller makes outgoing requests on a connection. It is a generated struct
> (e.g. `AdderClient`) that provides the same async methods as the service trait,
> and takes care of serialization and response handling internally.

> r[rpc.session-setup]
>
> When establishing a session, the user provides a handler for the root
> connection. The session returns a typed caller for the root connection,
> and a handle for accepting virtual connections.

In code, this looks like:

```rust
let (caller, accept_handle) = session
    .establish::<AdderClient>(my_adder_handler)
    .await?;

// caller is an AdderClient
let result = caller.add(3, 5).await?;
```

> r[rpc.virtual-connection.accept]
>
> When a virtual connection is opened by the counterpart, the accepting peer
> receives the connection metadata, decides which handler to assign to it,
> and obtains a typed caller for that virtual connection.

> r[rpc.virtual-connection.open]
>
> A peer may open a virtual connection on an existing session, providing a
> handler and receiving a typed caller, just like during session establishment.

# Requests and responses

> r[rpc.request]
>
> A `Request` message carries:
>
>   * A request ID, unique within the connection, allocated by the caller
>     using the connection's parity
>   * A method ID (see `r[rpc.method-id]`)
>   * Serialized arguments
>   * A list of channel IDs for channels that appear in the arguments,
>     allocated by the caller
>   * Metadata (key-value pairs for tracing, auth, deadlines, etc.)

> r[rpc.response]
>
> A `Response` message carries:
>
>   * The request ID of the request being responded to
>   * The serialized return value
>   * A list of channel IDs for channels that appear in the return type,
>     allocated by the callee
>   * Metadata

> r[rpc.request.id-allocation]
>
> Request IDs are allocated by the caller using the connection's parity.
> Sending a `Request` with an ID that does not match the caller's parity,
> or reusing an ID that is still in flight, is a protocol error.

> r[rpc.response.one-per-request]
>
> Every request MUST receive exactly one response. Sending a second response
> for the same request ID is a protocol error.

> r[rpc.unknown-method]
>
> If a handler receives a request with a method ID it does not recognize,
> it MUST send an error response indicating the method is unknown.
> This is a call-level error, not a protocol error — the connection
> remains open.

# Fallible methods

> r[rpc.fallible]
>
> A service method may return `T` (infallible) or `Result<T, E>` (fallible),
> where both `T` and `E` implement `Facet`.

> r[rpc.fallible.caller-signature]
>
> On the caller side, the generated client wraps all return types in
> `Result<_, RoamError<E>>`:
>
>   * Infallible `fn foo() -> T` becomes `fn foo() -> Result<T, RoamError>`
>   * Fallible `fn foo() -> Result<T, E>` becomes `fn foo() -> Result<T, RoamError<E>>`

> r[rpc.fallible.roam-error]
>
> `RoamError<E>` distinguishes application errors from protocol-level errors:
>
>   * `User(E)` — the handler ran and returned an application error
>   * `UnknownMethod` — no handler recognized the method ID
>   * `InvalidPayload` — the arguments could not be deserialized
>   * `Cancelled` — the call was cancelled before completion

> r[rpc.error.scope]
>
> Call errors affect only that call. The connection remains open and other
> in-flight requests are unaffected.

# Channels

> r[rpc.channel]
>
> A channel is a unidirectional, ordered sequence of typed values between
> two peers. At the type level, `Tx<T, N>` and `Rx<T, N>` indicate direction
> and initial credit. `T` is the element type; `N` is a `usize` const generic
> specifying how many items the sender may send before receiving explicit
> credit (see `r[rpc.flow-control.credit.initial]`). Each channel has exactly
> one sender and one receiver.

> r[rpc.channel.direction]
>
> `Tx<T, N>` means "I send" and `Rx<T, N>` means "I receive", where "I" is
> whoever holds the handle. Position determines who holds it:
>
>   * In arg position (handler holds): `Tx<T>` = handler sends → caller,
>     `Rx<T>` = handler receives ← caller.
>   * In return position (caller holds): `Tx<T>` = caller sends → handler,
>     `Rx<T>` = caller receives ← handler.

> r[rpc.channel.placement]
>
> `Tx<T, N>` and `Rx<T, N>` may appear in both argument types and return
> types of service methods. They MUST NOT appear in the error variant of a
> `Result` return type.

> r[rpc.channel.no-collections]
>
> `Tx<T, N>` and `Rx<T, N>` MUST NOT appear inside collections (lists,
> arrays, maps, sets). They may be nested arbitrarily deep inside structs
> and enums.

> r[rpc.channel.allocation]
>
> Channel IDs are allocated using the connection's parity. The caller
> allocates IDs for channels that appear in the request arguments. The
> callee allocates IDs for channels that appear in the response return type.

> r[rpc.channel.lifecycle]
>
> Channels are created as part of a request or response, but they outlive
> both. A channel remains live until it is explicitly closed or reset,
> or until the connection is torn down.

> r[rpc.channel.item]
>
> A `ChannelItem` message carries a channel ID and a serialized value of
> the channel's element type.

> r[rpc.channel.close]
>
> The sender of a channel sends `CloseChannel` when it is done sending.
> After sending `CloseChannel`, the sender MUST NOT send any more
> `ChannelItem` messages on that channel.

> r[rpc.channel.reset]
>
> The receiver of a channel sends `ResetChannel` to ask the sender to
> stop sending. After receiving `ResetChannel`, the sender MUST stop
> sending `ChannelItem` messages on that channel.

# Flow control

> r[rpc.flow-control]
>
> Roam provides backpressure at two levels: request pipelining limits and
> per-channel credit-based flow control.

## Request limits

> r[rpc.flow-control.max-concurrent-requests]
>
> Each connection has a per-direction limit on the number of concurrent
> in-flight requests. Each peer advertises the maximum number of requests
> it is willing to accept on a connection. A peer MUST NOT send a new
> request if it would exceed the counterpart's advertised limit.

> r[rpc.flow-control.max-concurrent-requests.default]
>
> The default limit is carried in `ConnectionSettings`, which is embedded
> in `Hello` (for the root connection) and `OpenConnection` (for virtual
> connections). See `r[session.connection-settings]`.

## Channel credit

> r[rpc.flow-control.credit]
>
> Channels use item-based credit for flow control. The receiver of a
> channel controls how many items the sender may send by granting credit.
> The sender MUST NOT send a `ChannelItem` if it has zero credit. Each
> sent `ChannelItem` consumes one unit of credit.

> r[rpc.flow-control.credit.initial]
>
> Initial credit is part of the channel's type signature. `Tx<T, N>` and
> `Rx<T, N>` carry a const generic `N: usize` that specifies the initial
> credit for the channel. When a channel is created (as part of a request
> or response), the sender starts with `N` units of credit. This value
> is known at compile time and is part of the signature hash, so both
> peers always agree on it.

> r[rpc.flow-control.credit.initial.zero]
>
> `N = 0` is valid. The sender MUST wait for an explicit `GrantCredit`
> before sending any items. This is useful for channels where the receiver
> needs full control over when data starts flowing.

> r[rpc.flow-control.credit.grant]
>
> The receiver of a channel sends a `GrantCredit` message to add credit.
> `GrantCredit` carries a connection ID, a channel ID, and an `additional`
> count (u32). The sender's available credit increases by `additional`.
> The receiver MAY send `GrantCredit` at any time after the channel exists.

> r[rpc.flow-control.credit.grant.additive]
>
> Credit is strictly additive. There is no mechanism to revoke granted
> credit. The receiver controls flow by choosing when and how much credit
> to grant.

> r[rpc.flow-control.credit.exhaustion]
>
> When the sender's credit reaches zero, it MUST stop sending `ChannelItem`
> messages on that channel until more credit is granted. The sender SHOULD
> apply backpressure to the producing code (e.g. by blocking a `send()`
> call) rather than buffering unboundedly.

# Cancellation

> r[rpc.cancel]
>
> A caller may send `CancelRequest` to indicate it is no longer interested
> in the response. The handler SHOULD stop processing the request, but
> a response may still arrive — the caller MUST be prepared to ignore it.

> r[rpc.cancel.channels]
>
> Cancelling a request does not automatically close or reset any channels
> that were created as part of that request. Channels have independent
> lifecycles and MUST be closed or reset explicitly.

# Pipelining

> r[rpc.pipelining]
>
> Multiple requests MAY be in flight simultaneously on a connection. Each
> request is independent; a slow or failed request MUST NOT block other
> requests.

# Metadata

> r[rpc.metadata]
>
> Requests and Responses carry metadata: a list of `(key, value, flags)`
> triples for out-of-band information such as tracing context, authentication
> tokens, or deadlines.

> r[rpc.metadata.value]
>
> A metadata value is one of three types:
>
>   * `String` — a UTF-8 string
>   * `Bytes` — an opaque byte buffer
>   * `U64` — a 64-bit unsigned integer

> r[rpc.metadata.flags]
>
> Each metadata entry carries a `u64` flags bitfield that controls handling
> behavior. Unknown flag bits MUST be preserved when forwarding metadata,
> but MUST be ignored for handling decisions.
>
> | Bit | Name | Meaning |
> |-----|------|---------|
> | 0 | `SENSITIVE` | See `r[rpc.metadata.flags.sensitive]` |
> | 1 | `NO_PROPAGATE` | See `r[rpc.metadata.flags.no-propagate]` |
> | 2–63 | Reserved | MUST be zero when creating; MUST be preserved when forwarding |

> r[rpc.metadata.flags.sensitive]
>
> When the `SENSITIVE` flag (bit 0) is set, the value MUST NOT be logged,
> traced, or included in error messages. Implementations MUST take care
> not to expose sensitive values in debug output, telemetry, or crash reports.

> r[rpc.metadata.flags.no-propagate]
>
> When the `NO_PROPAGATE` flag (bit 1) is set, the value MUST NOT be
> forwarded to downstream calls. A proxy or middleware that forwards
> metadata MUST strip entries with this flag set.

> r[rpc.metadata.keys]
>
> Metadata keys are case-sensitive UTF-8 strings. By convention, keys
> use lowercase kebab-case (e.g. `authorization`, `trace-parent`,
> `request-deadline`).

> r[rpc.metadata.duplicates]
>
> Duplicate keys are allowed. When multiple entries share the same key,
> all values MUST be preserved in order.

> r[rpc.metadata.unknown]
>
> Unknown metadata keys MUST be ignored — they MUST NOT cause errors
> or protocol violations.

### Examples

Authentication tokens should be marked sensitive to prevent logging:

```rust
metadata.push((
    "authorization".into(),
    MetadataValue::String("Bearer sk-...".into()),
    MetadataFlags::SENSITIVE,
));
```

Session tokens that shouldn't leak to downstream services:

```rust
metadata.push((
    "session-id".into(),
    MetadataValue::String(session_id),
    MetadataFlags::SENSITIVE | MetadataFlags::NO_PROPAGATE,
));
```

# Channel binding

> r[rpc.channel.discovery]
>
> Channel IDs in `Request.channels` and `Response.channels` MUST be listed
> in the order produced by a schema-driven traversal of the argument types
> (for requests) or return type (for responses). The traversal visits struct
> fields and active enum variant fields in declaration order. It does not
> descend into collections, since channels MUST NOT appear there (see
> `r[rpc.channel.no-collections]`). Channels inside an `Option` that is
> `None` at runtime are simply absent from the list.

> r[rpc.channel.payload-encoding]
>
> `Tx<T, N>` and `Rx<T, N>` values in the serialized payload MUST be encoded as
> unit placeholders. The actual channel IDs are carried out-of-band in the
> `channels` field of the `Request` or `Response` message.

> r[rpc.channel.binding]
>
> On the callee side, implementations MUST use the channel IDs from
> `Request.channels` as authoritative, patching them into deserialized
> argument values before binding streams. On the caller side, implementations
> MUST use the channel IDs from `Response.channels` to bind return-type
> channel handles. This separation enables transparent proxying: a proxy
> can forward `Request` and `Response` messages without parsing payloads.

## Channel pairs and shared state

> r[rpc.channel.pair]
>
> `channel<T>()` returns a `(Tx<T>, Rx<T>)` pair that share a single
> channel core. Both handles hold an `Arc` reference to the core. The
> core contains a `OnceLock<ChannelBinding>` where `ChannelBinding` is
> either a `Sink` or a `Receiver` — never both.

> r[rpc.channel.pair.binding-propagation]
>
> When the framework binds a channel handle that is part of a pair
> (created via `channel()`), the binding is stored in the shared core.
> The paired handle — which the caller or callee kept — reads or takes
> the binding from the same core. This allows the framework to bind
> both ends by touching only the handle that appears in the args or
> return value.

## Caller-side binding (args)

> r[rpc.channel.binding.caller-args]
>
> When the caller sends a request containing channel handles in the
> arguments, the framework iterates the channel locations from the
> `RpcPlan`, allocates a channel ID for each, and binds the handle
> in the args tuple. Channel IDs are collected into `Request.channels`.

> r[rpc.channel.binding.caller-args.rx]
>
> For an `Rx<T>` in arg position: the handler will receive, so the
> caller must send. The framework allocates a channel ID and creates
> a sink (via `ChannelBinder::create_tx`). The sink is stored in the
> shared core so the caller's paired `Tx<T>` can send through it.

> r[rpc.channel.binding.caller-args.tx]
>
> For a `Tx<T>` in arg position: the handler will send, so the caller
> must receive. The framework allocates a channel ID and creates a
> receiver (via `ChannelBinder::create_rx`). The receiver is stored
> in the shared core so the caller's paired `Rx<T>` can receive from it.

## Callee-side binding (args)

> r[rpc.channel.binding.callee-args]
>
> When the callee receives a request, channel handles in the deserialized
> arguments are standalone (not part of a pair). The framework iterates
> the channel locations from the `RpcPlan` and binds each handle directly
> using the channel IDs from `Request.channels`.

> r[rpc.channel.binding.callee-args.rx]
>
> For an `Rx<T>` in arg position: the handler receives. The framework
> calls `ChannelBinder::register_rx` with the channel ID to register
> the channel for routing and stores the receiver directly in the
> `Rx`'s receiver slot.

> r[rpc.channel.binding.callee-args.tx]
>
> For a `Tx<T>` in arg position: the handler sends. The framework
> calls `ChannelBinder::bind_tx` with the channel ID and stores the
> sink directly in the `Tx`'s sink slot.

## Callee-side binding (return)

> r[rpc.channel.binding.callee-return]
>
> When the callee returns a response containing channel handles, those
> handles were created via `channel()` and are part of a pair. The
> framework iterates the channel locations in the return type's `RpcPlan`,
> allocates a channel ID for each, and binds the handle in the return
> value. Channel IDs are collected into `Response.channels`.

> r[rpc.channel.binding.callee-return.rx]
>
> For an `Rx<T>` in return position: the caller will receive, so the
> callee must send. The framework allocates a channel ID and creates
> a sink. The sink is stored in the shared core so the callee's
> paired `Tx<T>` can send through it.

> r[rpc.channel.binding.callee-return.tx]
>
> For a `Tx<T>` in return position: the caller will send, so the
> callee must receive. The framework allocates a channel ID and creates
> a receiver. The receiver is stored in the shared core so the callee's
> paired `Rx<T>` can receive from it.

## Caller-side binding (return)

> r[rpc.channel.binding.caller-return]
>
> When the caller receives a response containing channel handles, those
> handles are standalone (deserialized from the response). The framework
> iterates the channel locations in the return type's `RpcPlan` and binds
> each handle directly using the channel IDs from `Response.channels`.

> r[rpc.channel.binding.caller-return.rx]
>
> For an `Rx<T>` in return position: the caller receives. The framework
> calls `ChannelBinder::register_rx` with the channel ID and stores the
> receiver directly in the `Rx`'s receiver slot.

> r[rpc.channel.binding.caller-return.tx]
>
> For a `Tx<T>` in return position: the caller sends. The framework
> calls `ChannelBinder::bind_tx` with the channel ID and stores the
> sink directly in the `Tx`'s sink slot.

## Handle hot path

> r[rpc.channel.pair.tx-read]
>
> `Tx::send` reads the sink from the shared core. If the `Tx` was
> created standalone (deserialized), it reads from its local sink slot.
> If it was created via `channel()`, it reads from the shared core's
> `ChannelBinding::Sink`.

> r[rpc.channel.pair.rx-take]
>
> `Rx::recv` takes the receiver on first call. If the `Rx` was created
> standalone (deserialized), the receiver is already in its local slot.
> If it was created via `channel()`, the first `recv` call takes the
> receiver from the shared core's `ChannelBinding::Receiver` into the
> local slot. Subsequent calls use the local slot directly.
