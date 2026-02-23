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
> two peers. At the type level, `Tx<T>` and `Rx<T>` indicate direction.
> Each channel has exactly one sender and one receiver.

> r[rpc.channel.direction]
>
> Service definitions are written from the handler's perspective:
>
>   * `Tx<T>` — data flows from handler to caller (the handler sends)
>   * `Rx<T>` — data flows from caller to handler (the caller sends)

> r[rpc.channel.placement]
>
> `Tx<T>` and `Rx<T>` may appear in both argument types and return types
> of service methods. They MUST NOT appear in the error variant of a
> `Result` return type.

> r[rpc.channel.no-collections]
>
> `Tx<T>` and `Rx<T>` MUST NOT appear inside collections (lists, arrays,
> maps, sets). They may be nested arbitrarily deep inside structs and enums.

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
> fields and active enum variant fields in declaration order. It MUST NOT
> descend into collections (lists, arrays, maps, sets). Channels inside an
> `Option` that is `None` at runtime are simply absent from the list.

> r[rpc.channel.payload-encoding]
>
> `Tx<T>` and `Rx<T>` values in the serialized payload MUST be encoded as
> unit placeholders. The actual channel IDs are carried out-of-band in the
> `channels` field of the `Request` or `Response` message.

> r[rpc.channel.binding]
>
> On the handler side, implementations MUST use the channel IDs from
> `Request.channels` as authoritative, patching them into deserialized
> argument values before binding streams. On the caller side, implementations
> MUST use the channel IDs from `Response.channels` to bind return-type
> channel handles. This separation enables transparent proxying: a proxy
> can forward `Request` and `Response` messages without parsing payloads.
