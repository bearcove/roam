+++
title = "roam specification"
description = "Formal roam RPC protocol specification"
weight = 10
+++

# Introduction

*This is roam specification v5.0.0, last updated February 22, 2026. It
canonically lives at <https://github.com/bearcove/roam> — where you can get the
latest version.*

roam is a **Rust-native** RPC protocol. There is no independent schema language;
Rust traits *are* the schema. Implementations for other languages (Swift,
TypeScript, etc.) are generated from Rust definitions.

## Defining a service

An application named `fantastic` would typically define services in `*-proto`.
Crates, if it has only one, the `fantastic-proto` crate would contain something
like:

```rust
#[roam::service]
pub trait Adder {
    /// Load a template by name.
    async fn add(&self, l: u32, r: u32) -> u32;
}
```

Proto crates are meant to only contain types and trait definitions (as much as
possible, modulo orphan rules) so that they may be joined with roam codegen to
generate client and server code for Swift and TypeScript.

All types that occur as arguments or in return position must implement the
`Facet` trait, from the [facet](https://docs.rs/facet) crate.

## Implementing a service

Given an `Adder` trait, the `roam::service` proc macro generates a trait
also named `Adder`, but with an added `&Context` parameter in first position:

```rust
#[derive(Clone)]
struct AdderHandler;

impl Adder for AdderHandler {
    /// Add two numbers.
    async fn add(&self, _cx: &Context, l: u32, r: u32) -> u32 {
        // we could fetch metadata etc. through `_cx`
        l + r
    };
}
```

## Consuming a service

The proc macro also generates a `{ServiceName}Client` struct, which provides the
same async methods, without `&Context` this time: 

```rust
// Make a call
let result = client.add(3, 5).await;
assert_eq!(result, 8);
```

...because metadata can be passed to the future, before awaiting it:

```rust
// Make a call with custom metadata
let result = client
    .add(3, 5)
    .with_metadata(meta)
    .await;
assert_eq!(result, 8);
```

## High-level concepts

But how do you obtain a client?

To "handle" a call (ie. send a response to an incoming request), or to "make" a
call (ie. send a request to the peer, expecting a response), one needs an active
connection.

Connections are the fourth layer in the roam connectivity model. First, we need to
establish a **Link** with the other peer: typically by accepting a TCP/Unix socket
connection, or establishing one, or negotiating an SHM link over a file and some
sockets, etc. etc. — this lets us exchange **Payloads** (opaque byte buffers).

On top of the **Link** is the **Wire** which deals with serialization and
deserialization to and from postcard.

On top of the **Wire** sits the **Session**: it has a stable identifiers, can be
resumed if we lose connectivity and have to re-create a new **Link**.

Finally, a **Session** can host many connections, starting with the root
connection, with identifier 0, which is always open.

So, the model goes:

  * Link (Memory, stdio, TCP, Unix sockets, Named pipes, WebSocket, SHM)
  * Wire (serialization/deserialiation)
  * Session (durable set of connections, request state machine etc.)
  * Connections (namespace for request/channel IDs)
  
Transports ("kinds of links") typically let you both "accept" or "connect".
In both cases, one must specify:

  * A service handler (to handle incoming requests)
  * A client type (to make outgoing requests)
  
```rust
// Simple case: connect over TCP, we act as a client only (don't handle any), brand new link
let (session, caller_a) = Session::new::<Adder>().connect_over(Tcp::new("127.0.0.1:3030")).build().await?;
let eight = caller_a.add(3, 5).await;

// Let's make a new connection in the same session - this time we handle requests on Echo
let caller_b = session.connect::<Adder>().handler<Echo>(EchoHandler::new()).build().await?;
let twelve = caller_a.add(6, 6).await;

// Now let's accept connections - we need a loop for that
let mut acceptor = Session::accept_over(Tcp::bind("127.0.0.1:3030")).await?;
loop {
    let incoming = acceptor.next().await?;
    
    let (session, caller_c) = incoming.accept::<Adder>().handler<Third>(ThirdHandler::new()).build().await?;
    // Handle this connection...
}

// TODO: add examples for websocket, SHM, etc. - what if you already have an established TCP socket?
// what if you have a custom transport? How does reconnection work here? How does reconnection work on the
// server (acceptor) side?
```

Important: `incoming.accept()`, `session.connect`, `Session::new` all default to
`ClientService = ()`, which impl client and handler for an empty service. 

## Codegen for third-party languages

Bindings for other languages (Swift, TypeScript) are generated using
a Rust codegen package which is linked together with the "proto" crate to
output Swift/TypeScript packages.

For examples of Swift usage, see the [vixen](https://github.com/bearcove/vixen)
build system. For examples of TypeScript, well, no active projects use it right
now.

# Core Semantics

This section defines transport-agnostic semantics that all roam
implementations MUST follow. Transport bindings (networked, SHM) encode
these concepts differently but preserve the same meaning.

## Identifiers

This specification uses small, typed identifiers. Unless otherwise stated,
all identifiers are scoped to a specific link or virtual connection as
described in the surrounding rules.

```rust
// Conceptual newtypes used by this specification.
pub struct SessionId(pub u32);
pub struct ConnectionId(pub u32);

/// Parity selects a partition of the u32 ID space.
///
/// Odd IDs: 1, 3, 5, ...
/// Even IDs: 2, 4, 6, ...
pub enum Parity {
    Odd,
    Even,
}

// Call- and channel-scoped identifiers (unified with SHM).
pub struct RequestId(pub u32);
pub struct ChannelId(pub u32);

// Blake3 Hash of various things about the method
pub struct MethodId(pub u64);

// Channel sequencing for exactly-once delivery.
pub struct Seq(pub u64);

// Opaque byte slice, postcard-serialized
pub struct Payload(pub Vec<u8>);

// Opaque capability used to authorize resuming a SessionId.
pub struct ResumeToken(pub [u8; 16]);
```

## Transports, Links, and Connections

A **transport** is a mechanism for communication (TCP, WebSocket, SHM, etc.).
A **link** is an instance of a transport between two **peers** — the actual
connection over which messages flow.

A link carries exactly one **session**. A session is the stable, resumable
communication context with its own call and channel state (connections, request
and channel ID spaces, dispatcher assignments, flow-control state).

A session multiplexes one or more **connections** identified by a `conn_id`.
Connection 0 is created implicitly when the link is established.

> r[core.link]
>
> A link is a bidirectional communication channel between two peers,
> established via a specific transport. The link carries exactly one
> roam Session.

> r[core.link.connection-zero]
>
> Connection 0 is established implicitly when the link is created.
> Both peers can immediately send messages on connection 0 after the
> Hello/HelloYourself exchange.

> r[core.session]
>
> Each session has a stable `session_id` (`SessionId`) that identifies the
> logical session across reconnects. A `session_id` is paired with a
> `resume_token` (`ResumeToken`) which is required to resume the session after
> a link failure.
>
> When a session is resumed on a new link, the session's connection IDs and
> all request/channel ID spaces continue; only the underlying transport link
> changes.

## Virtual Connections

Either peer may open additional virtual connections on the link.
This enables multiplexing: a proxy can map each downstream client to
a separate upstream connection, preserving session identity.

> r[core.conn.open]
>
> A peer opens a new connection by allocating a fresh `conn_id` and sending
> `Connect` on that `conn_id`. The remote peer responds with `Accept` or
> `Reject` on the same `conn_id`.

> r[core.conn.accept-required]
>
> The remote peer MUST have called `take_incoming_connections()` on
> connection 0 to accept new connections. If not listening, the peer
> MUST respond with `Reject`.

> r[core.conn.id-allocation]
>
> Connection IDs are allocated by the peer opening the connection (the sender
> of `Connect`). A `conn_id` MUST NOT be 0 and MUST NOT be reused within a
> session.

> r[core.conn.id-allocation.parity]
>
> Connection IDs greater than 0 are partitioned by parity (odd/even) so both
> peers can open connections concurrently without collisions:
>
> - Each peer has a session-level `Parity` negotiated during Hello.
> - A peer MUST allocate only `conn_id > 0` values matching its session
>   parity.
> - If a peer receives `Connect` using a `conn_id > 0` that matches the
>   receiver's session parity, it MUST treat it as a protocol error (send
>   Goodbye on connection 0 and close the link).

> r[core.conn.only-root-accepts]
>
> Only connection 0 (the root connection) can accept incoming connections.
> Virtual connections opened via `Connect`/`Accept` cannot themselves
> accept further nested connections.

> r[core.conn.lifecycle]
>
> A virtual connection is closed when either peer sends `Goodbye` on
> that connection. Closing a session terminates all in-flight calls and
> channels on that session and prevents future resumption.

> r[core.conn.independence]
>
> Virtual connections are independent. An error or closure on one
> connection does not affect other connections on the same link.
> Each connection has its own:
> - Request ID spaces (one per caller on the connection)
> - Channel ID space
> - Dispatcher (service handler)

### Connection Messages

| Message | Sender | Meaning |
|---------|--------|---------|
| **Connect** | either | Request to open a new virtual connection |
| **Accept** | receiver of Connect | Connection accepted |
| **Reject** | receiver of Connect | Connection refused (not listening) |

## Calls

A **call** is a request/response exchange identified by a `request_id`
(`RequestId`).

> r[core.call]
>
> A call consists of exactly one Request and exactly one Response with
> the same `request_id`. The caller sends the Request; the callee sends
> the Response.

> r[core.call.request-id]
>
> Request IDs are scoped to the (connection, caller). Each peer maintains
> its own `RequestId` sequence for the calls it initiates on a connection.
> When initiating calls, a peer MUST allocate Request IDs by incrementing a
> `u32` counter modulo 2^32 (wrapping).

> r[core.call.request-id.parity]
>
> Within a given connection, each peer has an assigned `Parity`. When
> initiating calls on that connection, a peer MUST allocate `RequestId` values
> matching its parity partition (odd if the peer's parity is Odd; even if the
> peer's parity is Even).

### Call Messages

The following abstract messages relate to calls:

| Message | Sender | Meaning |
|---------|--------|---------|
| **Request** | caller | Initiate a call with `request_id`, `method_id`, and payload |
| **Response** | callee | Complete a call with result or error |
| **Cancel** | caller | Request that the callee abandon the call |

> r[core.call.cancel]
>
> Cancel is advisory. The callee MAY ignore it if the call is already
> complete. A Response may still arrive after Cancel is sent (either
> the completed result or a `Cancelled` error). Implementations MUST
> handle late Responses gracefully.

## Channels (Tx/Rx)

A **channel** is a unidirectional, ordered sequence of typed values. At the
type level, roam provides `Tx<T>` and `Rx<T>` to indicate direction.

> r[core.channel]
>
> Service definitions are written from the **callee/handler** perspective.
> `Tx<T>` represents data flowing from **callee to caller** (output).
> `Rx<T>` represents data flowing from **caller to callee** (input).
> Each has exactly one sender and one receiver.

On the wire, `Tx<T>` and `Rx<T>` are schema-level markers for channeling.
Channel IDs are carried out-of-band in Request/Response `channels` (and in
channel messages like Data/Close). The direction is determined by the
type, not the ID. See `r[channeling.type]` and `r[call.request.channels]`.

> r[core.channel.return-forbidden]
>
> `Tx<T>` and `Rx<T>` MUST NOT appear in return types. They may
> only appear in argument position. The return type is always a plain
> value (possibly `()` for methods that only produce output via Rx).

For bidirectional communication, use one Tx (input) and one Rx (output).

### Channel Messages

The following abstract messages relate to channels:

| Message | Sender | Meaning |
|---------|--------|---------|
| **Data** | channel sender | Deliver one value of type `T` (sequenced) |
| **Ack** | channel receiver | Acknowledge receipt of Data up to a sequence number |
| **Close** | caller (for `Rx<T>`) | End of channel (no more Data from caller) |
| **Reset** | either peer | Abort the channel immediately |

Ack supports exactly-once delivery and transparent reconnection by allowing a
sender to retransmit unacknowledged Data after a disconnect.

For `Rx<T>` (caller→callee), the caller sends Close when done sending.
After sending Close, the caller MUST NOT send more Data on that channel.
See `r[channeling.close]` for details.

For `Tx<T>` (callee→caller), the channel is implicitly closed when the
callee sends the Response. No explicit Close message is sent.
See `r[channeling.lifecycle.response-closes-pulls]`.

Reset forcefully terminates a channel. After sending or receiving Reset,
both peers MUST discard any pending data and consider it dead.

### Channel ID Allocation

Channel IDs must be unique within a connection (`r[channeling.id.uniqueness]`).
ID 0 is reserved (`r[channeling.id.zero-reserved]`). The **caller** allocates
all channel IDs for a call (`r[channeling.allocation.caller]`).

For peer-to-peer transports, channel ID allocation is governed by the
connection's negotiated `Parity`. See `r[channeling.id.parity]` for details.

### Channels and Calls

Channels are established via method calls. `Rx<T>` channels may outlive
the Response — the caller continues sending until they send Close.
`Tx<T>` channels are implicitly closed when Response is sent.
See `r[channeling.call-complete]` and `r[channeling.channels-outlive-response]`.

## Errors

### Call Errors

> r[core.error.roam-error]
>
> Call results are wrapped in `RoamError<E>` which distinguishes
> application errors from protocol errors:

| Variant | Meaning |
|---------|---------|
| `User(E)` | Application returned an error (method ran) |
| `UnknownMethod` | No handler for `method_id` |
| `InvalidPayload` | Could not deserialize request |
| `Cancelled` | Call was cancelled |

> r[core.error.call-vs-connection]
>
> Call errors affect only that call. The connection remains open.
> Multiple calls can be in flight, and one failing does not affect others.

### Connection Errors

> r[core.error.connection]
>
> Connection errors are unrecoverable protocol violations. The peer
> detecting the error MUST send a **Goodbye** message with a reason
> and close the connection.

Examples: duplicate request ID, data after Close, unknown channel ID.

> r[core.error.goodbye-reason]
>
> The Goodbye reason MUST contain the rule ID that was violated
> (e.g., `core.channel.close`), optionally followed by context.

## Metadata

> r[core.metadata]
>
> Requests and Responses carry metadata: a list of key-value pairs
> for out-of-band information (tracing, auth, deadlines, etc.).

Unknown metadata keys MUST be ignored (`r[call.metadata.unknown]`).
See the [Metadata](#metadata-1) section for complete details.

## Idempotency

Connection failures create uncertainty: did the server process the request
before the connection dropped?

If a session is successfully resumed (`r[core.session]`), peers can provide
exactly-once delivery by replaying and deduplicating transport units after a
disconnect. Channel element delivery additionally uses per-channel `seq` and
`Ack` (`r[core.channel]`, Channel Messages table).

Nonces are an optional, transport-agnostic mechanism for idempotency across
session loss (e.g. server restart, session expiry, or implementations that do
not support resumption).

> r[core.nonce]
>
> Clients MAY include a nonce in request metadata to enable idempotent
> delivery. The metadata key is `roam-nonce` and the value MUST be
> `MetadataValue::Bytes` containing exactly 16 bytes (128 bits).

> r[core.nonce.generation]
>
> Nonces MUST be generated using a cryptographically secure random source.
> UUIDv4 (random variant) is acceptable.

> r[core.nonce.uniqueness]
>
> Each logically distinct request MUST use a unique nonce. Retrying the
> same logical request (due to transport failure) MUST reuse the original
> nonce.

### Server Deduplication

> r[core.nonce.dedup]
>
> If a server receives a request with a nonce it has processed before
> (within its retention window), it MUST return the cached response
> without re-executing the method handler.

> r[core.nonce.retention]
>
> Servers implementing nonce deduplication MUST retain nonce→response
> mappings for at least 5 minutes. Servers MAY retain them longer.

> r[core.nonce.scope]
>
> Nonce uniqueness is scoped to the server (or logical service instance).
> The same nonce sent to different servers is not deduplicated.

> r[core.nonce.storage]
>
> Servers storing nonce→response mappings MUST protect them appropriately.
> Responses may contain sensitive data.

### Client Retry Behavior

> r[core.nonce.retry]
>
> When retrying a request due to transport failure (connection reset,
> timeout, etc.), clients MUST use the same nonce as the original request.

> r[core.nonce.new-request]
>
> For logically new requests (not retries), clients MUST generate a
> fresh nonce.

### Optional Feature

> r[core.nonce.optional]
>
> Nonces are optional. Requests without a `roam-nonce` metadata entry
> are processed normally without deduplication. Retrying such requests
> may cause duplicate execution.

> r[core.nonce.server-support]
>
> Servers are not required to implement nonce deduplication. Servers
> that do not support it MUST ignore the `roam-nonce` metadata key
> (per `r[call.metadata.unknown]`).

### Integration with Reconnecting Clients

> r[core.nonce.reconnect]
>
> Auto-reconnecting client implementations (see Reconnecting Client
> Specification) MAY automatically attach nonces to requests and
> reuse them on retry. This makes reconnection transparent to callers.

> r[core.nonce.channels]
>
> Nonces apply to the initial Request that establishes channels.
> If a session is resumed, channel streams are resumed using channel `seq`
> and `Ack` (`r[core.channel]`, Channel Messages table). If a session is not
> resumed, channels are terminated with the failed link.

## Topologies

Transports may have different topologies:

- **Peer-to-peer** (TCP, WebSocket, QUIC): Two peers, either can call.
- **Hub** (SHM Hub): One host, multiple peers. Routing is required.

The shared memory transport[^SHM-SPEC] specifies its topology separately.

---

# Transport Bindings

The following sections define how Core Semantics are encoded for specific
transport categories. Each binding specifies message encoding, framing,
connection establishment, and channel ID allocation.

## Service Definitions

A "proto crate" contains one or more "services" (Rust async traits) which
themselves contain one or more "methods" (not functions), which have parameters
and a return type:

```rust
// proto.rs

#[roam::service]
//└────┬────┘         Service definition
pub trait TemplateHost {
//         └────┬─────┘  Service name
    async fn load_template(&self, context_id: ContextId, name: String) -> LoadTemplateResult;
    //       └─────┬──────┘       └──────────────┬────────────────┘    └────────┬──────────┘
    //          Method                       Parameters                     Return type
}

// More services can be defined in the same proto crate...
```

## Method Identity

Every method has a unique 64-bit identifier derived from its service name,
method name, and signature. This is what gets sent on the wire in `Request`
messages.

The method ID ensures that:
- Different services can have methods with the same name without collision
- Changing a method's signature produces a different ID (incompatible)

Collisions are astronomically unlikely — the 64-bit hash space is large enough
that accidental collisions between legitimately different methods won't happen
in practice.

The exact algorithm for computing method IDs is defined in the
[^RUST-SPEC]. Other language
implementations receive pre-computed method IDs from code generation.

## Schema Evolution

Adding new methods to a service is always safe — peers that don't know about
a method will simply report it as unknown.

Most other changes are breaking:
- Renaming a service or method
- Changing argument types, order, or return type
- Changing the structure of any type used in the signature (field names, order, enum variants)
- Substituting container types (e.g., `Vec<T>` → `HashSet<T>`) — these have
  different signature tags even if wire-compatible at the POSTCARD level

Note: Argument *names* are not part of the wire format and can be changed
freely. Only types and their order matter.

## Error Scoping

Errors in roam have different scopes, from narrowest to widest:

**Application errors** are part of the method's return type. A method that
returns `Result<User, UserError>::Err(NotFound)` is a *successful* RPC —
the method ran and returned a value. These are not RPC errors.

**Call errors** mean an RPC failed, but only that specific call is affected.
Other in-flight calls and channels continue normally. Examples:
  * `UnknownMethod` — no handler for this method ID
  * `InvalidPayload` — couldn't deserialize the arguments
  * `Cancelled` — caller cancelled the request

**Channel errors** affect a single channel. The channel is closed but other
channels and calls are unaffected. A peer signals channel errors by sending
Reset.

**Connection errors** are protocol violations. The peer sends a Goodbye
message (citing the violated rule) and closes the connection. Everything
on this connection is torn down. Examples:
  * Data/Close/Reset on an unknown channel ID
  * Data after Close

# RPC Calls

An RPC call is a request/response exchange: one request, one response.
This section specifies the complete lifecycle.

## Request IDs

> r[call.request-id.uniqueness]
>
> Request IDs are scoped to the (connection, caller). Each peer maintains
> its own `RequestId` sequence for the calls it initiates on a connection.

> r[call.request-id.allocation]
>
> When initiating calls, a peer MUST allocate Request IDs by incrementing a
> `u32` counter modulo 2^32 (wrapping).

> r[call.request-id.liveness]
>
> A request is "live" from when the Request message is sent until the caller
> has received the corresponding Response.

> r[call.request-id.no-reuse-while-live]
>
> A caller MUST NOT reuse a live `RequestId` for a different logical call.
> Reusing a live RequestId is a connection error.

> r[call.request-id.wrap-window]
>
> Because `RequestId` wraps modulo 2^32, implementations MUST bound the live
> request window to be strictly less than 2^31. The negotiated
> `max_concurrent_requests` limit (`r[flow.request.concurrent-limit]`) MUST be
> enforced as part of this bound.

> r[call.request-id.serial-order]
>
> When an implementation needs to compare or advance `RequestId` values, it
> MUST use serial number arithmetic modulo 2^32: for two `u32` values `a` and
> `b`, `a` is considered "after" `b` if `(a - b) mod 2^32` is in `1..2^31`.

> r[call.request-id.cancel-still-live]
>
> Sending a Cancel message does NOT remove a request from live status. The
> request remains live until a Response is received.

For channeling methods, the Request/Response exchange negotiates channels,
but those channels have their own lifecycle independent of the call. See
[Channeling RPC](#channeling-rpc) for details.

## Initiating a Call

> r[call.initiate]
>
> A call is initiated by sending a Request message.

A Request contains:

```rust
Request {
    request_id: RequestId,
    method_id: MethodId,
    metadata: Metadata,
    channels: Vec<ChannelId>,  // Channel IDs used by this call, in declaration order
    payload: Payload,  // [^POSTCARD]-encoded arguments
}
```

> r[call.request.channels]
>
> The `channels` field MUST contain all channel IDs used by the call (both
> `Tx<T>` and `Rx<T>` parameters), in declaration order. This enables
> transparent proxying without parsing the payload.

> r[call.request.channels.schema-driven]
>
> Channel discovery is defined by the method schema, not by byte-by-byte
> inspection of payload values. Implementations MUST traverse only struct
> fields (including tuples) and active enum variant fields when collecting
> channel IDs. They MUST NOT traverse list or map container elements. For
> example, `Vec<T>` values (lists) and `HashMap<K, V>` values (maps) are not
> traversed for channel discovery.

> r[call.request.payload-encoding]
>
> The payload MUST be the [^POSTCARD] encoding of a tuple containing all
> method arguments in declaration order.

For example, a method `fn add(a: i32, b: i32) -> i64` with arguments `(3, 5)`
would have a payload that is the [^POSTCARD] encoding of the tuple `(3i32, 5i32)`.

## Completing a Call

> r[call.complete]
>
> A call is completed by sending a Response message with the same
> `request_id` as the original Request.

A Response contains:

```rust
Response {
    request_id: RequestId,
    metadata: Metadata,
    payload: Payload,  // [^POSTCARD]-encoded Result<T, RoamError<E>>
}
```

Where `T` is the method's success type and `E` is the method's error type
(if the method returns `Result<T, E>`).

## Response Encoding

> r[call.response.encoding]
>
> The response payload MUST be the [^POSTCARD] encoding of `Result<T, RoamError<E>>`,
> where `T` and `E` come from the method signature.

For a method declared as:

```rust
async fn get_user(&self, id: UserId) -> Result<User, UserError>;
```

The response payload is `Result<User, RoamError<UserError>>`.

For a method that cannot fail at the application level:

```rust
async fn ping(&self) -> Pong;
```

The response payload is `Result<Pong, RoamError<Infallible>>` (or an
equivalent encoding where the `User` variant cannot occur).

## Metadata

Requests and Responses carry a `metadata` field for out-of-band information.

> r[call.metadata.type]
>
> Metadata is a list of entries: `Vec<(String, MetadataValue, u64)>`.
> Each entry is a triple of (key, value, flags).

```rust
enum MetadataValue {
    String(String),  // 0
    Bytes(Vec<u8>),  // 1
    U64(u64),        // 2
}

// Metadata entry: (key, value, flags)
type Metadata = Vec<(String, MetadataValue, u64)>;
```

> r[call.metadata.flags]
>
> The flags field is a `u64` bitfield controlling metadata handling:
>
> | Bit | Name | Meaning |
> |-----|------|---------|
> | 0 | `SENSITIVE` | Value MUST NOT be logged, traced, or included in error messages |
> | 1 | `NO_PROPAGATE` | Value MUST NOT be forwarded to downstream calls |
> | 2-63 | Reserved | MUST be zero; peers MUST ignore unknown flag bits |
>
> Flags are encoded as a varint, so common values (0, 1, 2, 3) use only 1 byte.

> r[call.metadata.keys]
>
> Metadata keys are case-sensitive strings. Keys MUST be at most 256
> bytes (UTF-8 encoded).

> r[call.metadata.duplicates]
>
> Duplicate keys are allowed. If multiple entries have the same key,
> all values are preserved in order. Consumers MAY use any of the values
> (typically the first or last).

> r[call.metadata.order]
>
> Metadata order MUST be preserved during transmission. Order is not
> semantically meaningful for most uses, but some applications may
> rely on it (e.g., multi-value headers).

> r[call.metadata.unknown]
>
> Unknown metadata keys MUST be ignored.

> r[call.metadata.limits]
>
> Metadata limits:
> - At most 128 metadata entries (key-value pairs)
> - Each key at most 256 bytes
> - Each value at most 16 KB (16,384 bytes)
> - Total metadata size at most 64 KB (65,536 bytes)
>
> If a peer receives a message exceeding these limits, it MUST send a
> Goodbye message (reason: `call.metadata.limits`) and close the
> connection.

### Example Uses

Metadata is application-defined. Common uses include:

- **Deadlines**: Absolute timestamp after which the caller no longer cares
- **Distributed tracing**: W3C traceparent/tracestate, or other trace IDs
- **Authentication**: Bearer tokens, API keys, signatures (with `SENSITIVE` flag)
- **Priority**: Scheduling hints for request processing order
- **Compression**: Indicating payload compression scheme

### Flag Usage Examples

Authentication tokens should be marked sensitive to prevent logging:

```rust
metadata.push((
    "authorization".to_string(),
    MetadataValue::String("Bearer sk-...".to_string()),
    SENSITIVE,  // bit 0 = don't log this value
));
```

Session tokens that shouldn't leak to downstream services:

```rust
metadata.push((
    "session-id".to_string(),
    MetadataValue::String(session_id),
    SENSITIVE | NO_PROPAGATE,  // bits 0+1 = don't log, don't forward
));
```

## RoamError

> r[call.error.roam-error]
>
> `RoamError<E>` distinguishes application errors from protocol errors.
> The variant order defines wire discriminants ([^POSTCARD] varint encoding):

| Discriminant | Variant | Payload | Meaning |
|--------------|---------|---------|---------|
| 0 | `User` | `E` | Application returned an error |
| 1 | `UnknownMethod` | none | No handler for this `method_id` |
| 2 | `InvalidPayload` | none | Could not deserialize request arguments |
| 3 | `Cancelled` | none | Caller cancelled the request |

In Rust syntax (for clarity):

```rust
enum RoamError<E> {
    User(E),         // 0
    UnknownMethod,   // 1
    InvalidPayload,  // 2
    Cancelled,       // 3
}
```

> r[call.error.user]
>
> The `User(E)` variant (discriminant 0) carries the application's error
> type. This is semantically different from protocol errors — the method
> ran and returned `Err(e)`.

> r[call.error.protocol]
>
> Discriminants 1-3 are protocol-level errors. The method may not have
> run at all (UnknownMethod, InvalidPayload) or was interrupted
> (Cancelled).

This design means callers always know: "Did my application logic fail,
or did the RPC infrastructure fail?"

### Returning Call Errors

> r[call.error.unknown-method]
>
> If a callee receives a Request with a `method_id` it does not recognize,
> it MUST send a Response with `Err(RoamError::UnknownMethod)`. The
> connection remains open.

> r[call.error.invalid-payload]
>
> If a callee cannot deserialize the Request payload, it MUST send a
> Response with `Err(RoamError::InvalidPayload)`. The connection
> remains open.

## Call Lifecycle

The complete lifecycle of an RPC call:

```aasvg
.--------.                                        .--------.
| Caller |                                        | Callee |
'---+----'                                        '---+----'
    |                                                 |
    +-------- Request(id=1, method, payload) -------->|
    |                                                 |
    |                                      [execute handler]
    |                                                 |
    |<------- Response(id=1, Ok(payload)) ------------+
    |                                                 |
```

> r[call.lifecycle.single-response]
>
> For each Request, the callee MUST send exactly one Response with the
> same `request_id`. No more, no less.

> r[call.lifecycle.ordering]
>
> Responses MAY arrive in any order. The caller MUST use `request_id`
> for correlation, not arrival order.

> r[call.response.unknown-request-id]
>
> If a caller receives a Response with a `request_id` that does not match
> any in-flight request, it MUST treat this as a protocol violation:
> send `Goodbye` on connection 0 (reason: `call.response.unknown-request-id`)
> and close the link.

> r[call.response.stale-timeout]
>
> Implementations MAY enforce a timeout for pending responses. If the timeout
> is exceeded, they MUST fail pending calls with a connection error, send
> `Goodbye` on connection 0 (reason: `call.response.stale-timeout`), and close
> the link.

## Cancellation

```rust
Cancel {
    request_id: RequestId,  // The request to cancel
}
```

> r[call.cancel.message]
>
> A caller MAY send a Cancel message to request that the callee stop
> processing a request. The Cancel message MUST include the `request_id`
> of the request to cancel.

> r[call.cancel.best-effort]
>
> Cancellation is best-effort. The callee MAY have already completed the
> request, or MAY be unable to cancel in-progress work. The callee MUST
> still send a Response (either the completed result or `Cancelled` error).

> r[call.cancel.no-response-required]
>
> The caller MUST NOT wait indefinitely for a response after sending Cancel.
> Implementations MAY use a timeout after which the caller considers the
> request cancelled locally, even without a response.

## Pipelining

> r[call.pipelining.allowed]
>
> Multiple requests MAY be in flight simultaneously. The caller does not
> need to wait for a response before sending the next request.

> r[call.pipelining.independence]
>
> Each request is independent. A slow or failed request MUST NOT block
> other requests.

This enables efficient batching — a caller can send 10 requests, then
await all 10 responses, rather than round-tripping each one sequentially.

# Channeling RPC

Channeling methods have `Rx<T>` (caller→callee) or `Tx<T>` (callee→caller)
in argument position. Unlike simple RPC calls, data flows continuously over dedicated
channels.

## Tx and Rx Types

> r[channeling.type]
>
> `Tx<T>` and `Rx<T>` are roam-provided types recognized by the
> `#[roam::service]` macro. Channel IDs are carried out-of-band in Request/Response
> `channels` (`r[call.request.channels]`). The Request payload MUST NOT contain
> channel ID information (`r[channeling.allocation.caller]`).

> r[channeling.caller-pov]
>
> Service definitions are written from the **callee/handler** perspective.
> `Rx<T>` means the handler receives a stream of `T` values from the caller.
> `Tx<T>` means the handler sends a stream of `T` values to the caller.

Example:

```rust
// Service definition (callee/handler perspective)
#[roam::service]
pub trait Channeling {
    async fn sum(&self, numbers: Rx<u32>) -> u32;       // caller→callee
    async fn range(&self, n: u32, output: Tx<u32>);     // callee→caller
}

// Generated caller stub — same types as definition
impl ChannelingClient {
    async fn sum(&self, numbers: Rx<u32>) -> u32;       // caller sends
    async fn range(&self, n: u32, output: Tx<u32>);     // caller receives
}
```

The number of channels in a call is not always obvious from the method
signature — they may appear inside enums, so the actual IDs present depend
on which variant is passed.

## Channel ID Allocation

> r[channeling.allocation.caller]
>
> The **caller** allocates ALL channel IDs (both Tx and Rx). Channel IDs
> are listed in the Request's `channels` field (see `r[call.request.channels]`),
> in declaration order.
> The callee does not allocate any IDs.
>
> The Request payload MUST NOT contain channel ID information. `Tx<T>` and
> `Rx<T>` values in the payload MUST be encoded as unit placeholders.
>
> On the server side, implementations MUST use the channel IDs from the
> `channels` field as authoritative, patching them into deserialized args
> before binding streams. This ensures transparent proxying can work without
> parsing the payload.

> r[channeling.id.uniqueness]
>
> A channel ID MUST be unique within a connection.

> r[channeling.id.zero-reserved]
>
> Channel ID 0 is reserved. If a peer receives a channel message with
> `channel_id` of 0, it MUST send a Goodbye message (reason:
> `channeling.id.zero-reserved`) and close the connection.

> r[channeling.id.parity]
>
> Each connection has a negotiated `Parity` that partitions `ChannelId` values
> between the two peers.
>
> When initiating a call on a connection, the caller MUST allocate all
> `ChannelId` values for that call from the caller's parity partition for that
> connection (odd if the caller's parity is Odd; even if it is Even).
>
> This prevents collisions when both peers make concurrent calls on the same
> connection.

## Call Lifecycle with Channels

### Caller Channeling (Tx): `sum(numbers: Tx<u32>) -> u32`

```
Caller                                  Callee
    |                                          |
    |-- Request(sum, tx=1) ------------------->|
    |-- Data(channel=1, 10) ------------------>|
    |-- Data(channel=1, 20) ------------------>|
    |-- Close(channel=1) --------------------->|
    |                                          |
    |<-- Response(Ok, 30) --------------------|
```

### Callee Channeling (Rx): `range(n, output: Rx<u32>)`

```
Caller                                  Callee
    |                                          |
    |-- Request(range, n=3, rx=1) ------------>|
    |                                          |
    |<-- Data(channel=1, 0) -------------------|
    |<-- Data(channel=1, 1) -------------------|
    |<-- Data(channel=1, 2) -------------------|
    |<-- Response(Ok, ()) --------------------|  // rx channel implicitly closed
```

### Bidirectional: `pipe(input: Tx, output: Rx)`

```
Caller                                  Callee
    |                                          |
    |-- Request(pipe, tx=1, rx=3) ------------>|
    |-- Data(channel=1, "a") ----------------->|
    |<-- Data(channel=3, "a") -----------------|
    |-- Data(channel=1, "b") ----------------->|
    |<-- Data(channel=3, "b") -----------------|
    |-- Close(channel=1) --------------------->|
    |<-- Response(Ok, ()) --------------------|  // rx=3 closed
```

> r[channeling.lifecycle.immediate-data]
>
> The caller MAY send Data on `Tx<T>` channels immediately after sending
> the Request, without waiting for Response. This enables pipelining for
> lower latency.

> r[channeling.lifecycle.speculative]
>
> If the caller sends Data before receiving Response, and the Response
> is an error (`Err(RoamError::UnknownMethod)`, `Err(RoamError::InvalidPayload)`,
> etc.), the Data was wasted. The channel IDs are "burned" — they were
> never successfully opened and MUST NOT be reused.

> r[channeling.lifecycle.response-closes-pulls]
>
> When the callee sends Response, all `Rx<T>` channels are implicitly
> closed. The callee MUST NOT send Data on any Rx channel after sending Response.

> r[channeling.lifecycle.caller-closes-pushes]
>
> The caller MUST send Close on each `Tx<T>` channel when done sending.
> The callee waits for Close before it knows all input has arrived.

> r[channeling.error-no-channels]
>
> `Tx<T>` and `Rx<T>` MUST NOT appear inside error types. A method's
> error type `E` in `Result<T, E>` MUST NOT contain `Tx<T>` or `Rx<T>`
> at any nesting level.

## Channel Data Flow

> r[channeling.data]
>
> The sending peer sends Data messages containing [^POSTCARD]-encoded values
> of the channel's element type `T`. Each Data message contains exactly
> one value.

> r[channeling.data.size-limit]
>
> Each channel element MUST NOT exceed `max_payload_size` bytes (the same
> limit that applies to Request/Response payloads). If a peer receives
> a channel element exceeding this limit, it MUST send a Goodbye message
> (reason: `channeling.data.size-limit`) and close the connection.

> r[channeling.data.invalid]
>
> If a peer receives a Data message that cannot be deserialized as the
> channel's element type, it MUST send a Goodbye message (reason:
> `channeling.data.invalid`) and close the connection.

> r[channeling.close]
>
> For `Rx<T>` (caller→callee), the caller sends Close when done.
> For `Tx<T>` (callee→caller), the channel closes implicitly with Response.

> r[channeling.data-after-close]
>
> If a peer receives a Data message on a channel after it has been
> closed, it MUST send a Goodbye message (reason: `channeling.data-after-close`)
> and close the connection.

## Resetting a Channel

> r[channeling.reset]
>
> Either peer MAY send Reset to forcefully terminate a channel.
> The sender uses Reset to abandon early; the receiver uses Reset to signal
> it no longer wants data.

> r[channeling.reset.effect]
>
> Upon receiving Reset, the peer MUST consider the channel terminated.
> Any further Data, Close messages for that ID MUST be ignored
> (they may arrive due to race conditions).

> r[channeling.unknown]
>
> If a peer receives a channel message (Data, Close, Reset) with a
> `channel_id` that was never opened, it MUST send a Goodbye message
> (reason: `channeling.unknown`) and close the connection.

## Channels and Call Completion

> r[channeling.call-complete]
>
> The RPC call completes when the Response is received. At that point:
> - All `Tx<T>` channels are closed (callee can no longer send)
> - `Rx<T>` channels may still be open (caller may still be sending)
> - The request ID is no longer in-flight

> r[channeling.channels-outlive-response]
>
> `Rx<T>` channels (caller→callee) may outlive the Response. The caller
> continues sending until they send Close. The callee processes the final
> return value only after all input channels are closed.

# Messages

Everything roam does — method calls, channels, control signals — is
built on messages exchanged between peers.

```rust
enum Message {
    // Link handshake (no conn_id)
    Hello(Hello),
    HelloYourself(HelloYourself),
    
    // Virtual connection control (conn_id > 0)
    Connect { conn_id: ConnectionId, parity: Parity, metadata: Metadata },
    Accept { conn_id: ConnectionId, metadata: Metadata },
    Reject { conn_id: ConnectionId, reason: String, metadata: Metadata },

    // Connection control (conn_id scoped)
    Goodbye { conn_id: ConnectionId, reason: String },
    
    // RPC (conn_id scoped)
    Request { conn_id: ConnectionId, request_id: RequestId, method_id: MethodId, metadata: Metadata, channels: Vec<ChannelId>, payload: Payload },
    Response { conn_id: ConnectionId, request_id: RequestId, metadata: Metadata, payload: Payload },
    Cancel { conn_id: ConnectionId, request_id: RequestId },
    
    // Channels (conn_id scoped)
    Data { conn_id: ConnectionId, channel_id: ChannelId, seq: Seq, payload: Payload },
    Ack { conn_id: ConnectionId, channel_id: ChannelId, seq: Seq },
    Close { conn_id: ConnectionId, channel_id: ChannelId },
    Reset { conn_id: ConnectionId, channel_id: ChannelId },
}
```

> r[message.conn-id]
>
> All messages except `Hello` and `HelloYourself` include a `conn_id` field
> identifying which connection they belong to.
>
> If a peer receives a message with an unknown `conn_id`, it MUST treat it as a
> protocol error (send Goodbye on connection 0 and close the link), except that
> `Connect` is permitted to introduce a new `conn_id` (see `r[message.connect.conn-id]`).

Messages are [^POSTCARD]-encoded. The enum discriminant identifies the message
type, and each variant contains only the fields it needs.

> r[message.unknown-variant]
>
> If a peer receives a Message with an unknown enum discriminant, it
> MUST send a Goodbye message (reason: `message.unknown-variant`) and
> close the connection.

> r[message.decode-error]
>
> If a peer cannot decode a received message (invalid [^POSTCARD] encoding,
> length-prefix framing error, or malformed fields), it MUST send a Goodbye
> message (reason: `message.decode-error`) and close the connection.

## Message Types

### Hello

> r[message.hello.timing]
>
> The link initiator MUST send `Hello` immediately after link establishment,
> before any other message.
>
> The link acceptor MUST wait to receive `Hello` before sending
> `HelloYourself`.

> r[message.hello.structure]
>
> `Hello` and `HelloYourself` are enums to allow future versions.

> r[message.hello.unknown-version]
>
> If a peer receives `Hello` or `HelloYourself` with an unknown variant, it
> MUST send a Goodbye message (with reason containing
> `message.hello.unknown-version`) and close the link.

> r[message.hello.ordering]
>
> The initiator MUST NOT send any message other than `Hello` until it has
> received `HelloYourself`.
>
> The acceptor MUST NOT send any message other than `HelloYourself` until it
> has received `Hello`.

Hello and HelloYourself are versioned to allow future negotiation changes:

```rust
enum Hello {
    V7 {
        max_payload_size: u32,
        max_concurrent_requests: u32,
        parity: Parity,
    },
}

enum HelloYourself {
    V7 {
        max_payload_size: u32,
        max_concurrent_requests: u32,
    },
}
```

| Field | Description |
|-------|-------------|
| `max_payload_size` | Maximum bytes in a Request/Response payload |
| `max_concurrent_requests` | Maximum in-flight requests per connection |

> r[message.hello.negotiation]
>
> The effective limits for a session are the minimum of both peers'
> advertised values.

> r[message.hello.enforcement]
>
> If a peer receives a Request or Response whose payload exceeds the
> negotiated `max_payload_size`, it MUST send a Goodbye message
> (reason: `message.hello.enforcement`) and close the link.

> r[message.hello.parity]
>
> `Hello` includes a session-level `parity` used for allocating `ConnectionId`
> values greater than 0 and for allocating Request/Channel IDs on connection 0.
>
> - The initiator chooses `parity` and sends it in `Hello`.
> - The acceptor MUST use the opposite parity.
>
> Session resumption metadata is not carried by `Hello`/`HelloYourself`.
> (Resumption is handled by the reliability layer below the roam message model.)

### Connect / Accept / Reject

These messages manage virtual connections on a link.

```rust
Connect { conn_id: ConnectionId, parity: Parity, metadata: Metadata }
Accept { conn_id: ConnectionId, metadata: Metadata }
Reject { conn_id: ConnectionId, reason: String, metadata: Metadata }
```

> r[message.connect.initiate]
>
> Either peer MAY send `Connect` to request a new virtual connection.
> The connection is identified by the `conn_id` carried in the message, which
> MUST be unused in the session.

> r[message.connect.metadata]
>
> Connect metadata MAY include authentication tokens, routing hints,
> tracing context, or application-specific data. The same metadata
> limits apply as for RPC calls (see `r[call.metadata.limits]`).

> r[message.connect.conn-id]
>
> The `conn_id` used for a Connect MUST be greater than 0 and MUST match the
> sender's session-level parity (see `r[core.conn.id-allocation.parity]`).

> r[message.connect.parity]
>
> `Connect.parity` assigns the sender's parity for `RequestId` and `ChannelId`
> allocation inside that connection. The receiver MUST use the opposite parity
> inside that connection.

> r[message.connect.state]
>
> Before `Accept` is received, the sender MUST NOT send any message other than
> `Connect` on the new `conn_id`.

> r[message.accept.response]
>
> If the peer is listening for incoming connections (via
> `take_incoming_connections()` on connection 0), it responds with
> `Accept`. The new connection is immediately usable.

> r[message.accept.metadata]
>
> Accept metadata MAY include connection-specific configuration or tracing
> context.

> r[message.reject.response]
>
> If the peer is not listening for incoming connections, or if the
> connection is rejected for any other reason (auth failure, rate
> limiting, etc.), it MUST respond with `Reject`.

> r[message.reject.reason]
>
> The `reason` field MAY describe why the connection was rejected.
> Common reasons include: "not listening", "unauthorized", "rate limited".
> Metadata MAY provide additional structured rejection information.

> r[message.connect.timeout]
>
> If no Accept or Reject is received within a reasonable time,
> implementations MAY treat the Connect as failed. The `conn_id` MUST NOT be
> reused.

### Goodbye

Goodbye closes a virtual connection. The `conn_id` field specifies which
connection to close.

```rust
Goodbye { conn_id: ConnectionId, reason: String }
```

> r[message.goodbye.send]
>
> A peer MUST send a Goodbye message before closing a virtual connection
> due to a protocol error. The `reason` field MUST contain the rule ID
> that was violated (e.g., `channeling.id.zero-reserved`), optionally
> followed by additional context.

> r[message.goodbye.receive]
>
> Upon receiving a Goodbye message, a peer MUST stop sending messages
> on that connection. All in-flight requests on that connection fail
> with a connection error. All open channels on that connection are
> terminated. Other connections on the same link are unaffected.

> r[message.goodbye.connection-zero]
>
> Sending Goodbye on connection 0 closes the entire link. All
> virtual connections are terminated, and the underlying transport
> (TCP socket, WebSocket, etc.) is closed.

> r[message.goodbye.graceful]
>
> A peer MAY send Goodbye with an empty reason for graceful shutdown
> (not due to an error). This is the normal way to close a connection.

### Request / Response / Cancel

`Request` initiates an RPC call. `Response` returns the result. `Cancel`
requests that the callee stop processing a request.

The `request_id` correlates requests with responses, enabling multiple
calls to be in flight simultaneously (pipelining).

### Data / Close / Reset

`Data` carries payload bytes on a channel, identified by `channel_id`.
Each `Data` message carries a monotonically increasing `seq` number that is
scoped to `(conn_id, channel_id)`. `Ack` acknowledges receipt of `Data` up to a
sequence number, enabling retransmission of unacknowledged `Data` after
disconnect for exactly-once delivery.

`Close` signals end-of-channel — the sender is done (see `r[core.channel.close]`).
`Reset` forcefully terminates a channel.

# Transports

Different transports require different handling:

| Kind | Example | Framing | Channels |
|------|---------|---------|---------|
| Message | WebSocket | Transport provides | All in one |
| Multi-stream | QUIC | Per stream | Can map to transport streams |
| Byte stream | TCP | 4-byte length prefix | All in one |

## Message Transports

Message transports (like WebSocket) deliver discrete messages.

> r[transport.message.one-to-one]
>
> Each transport message MUST contain exactly one roam message,
> [^POSTCARD]-encoded. Fragmentation and reassembly are not supported.

> r[transport.message.binary]
>
> Transport messages MUST be binary (not text). For WebSocket, this
> means binary frames, not text frames.

> r[transport.message.multiplexing]
>
> All messages (control, RPC, channel data) flow through the same
> link. The `channel_id` and `conn_id` fields provide multiplexing.

## Byte Stream Transports

Byte stream transports (like TCP) provide a single ordered byte stream.

> r[transport.bytestream.length-prefix]
>
> Messages MUST be framed using a 4-byte little-endian length prefix
> followed by exactly that many message bytes.
> 
> ```
> [len: u32 LE][message bytes][len: u32 LE][message bytes]...
> ```

All messages flow through the single byte stream. The `channel_id` field
in channel messages provides multiplexing.

# Introduction

This document specifies implementation details for the Rust implementation of
roam. These details are NOT required for implementations in other languages
— Swift, TypeScript, etc. get their types and method identities from code
generated by the Rust proto crate.

# Layers

This section is prescriptive: it defines the target Rust architecture.

## Terms and Definitions

> rs[term.message]
>
> A **Message** is one roam protocol message (for example: `Request`,
> `Response`, `Data`, `Close`, `Credit`, `Goodbye`) as defined by the main
> specification's message model.
>
> Example: `Request { conn_id: 0, request_id: 42, ... }`.

> rs[term.payload]
>
> A **Payload** is an opaque byte buffer carried inside roam messages (for
> example: Request/Response payloads, or channel Data payload bytes).
> In Rust this is `roam_types::Payload` (a `#[repr(transparent)]` wrapper over
> `Vec<u8>`).
>
> A Payload is NOT the `Link` boundary item type in Rust: Rust `Link` is typed
> and uses `Codec` to encode/decode `T` directly.

> rs[term.transport]
>
> A **transport** is a mechanism that sends and receives roam `Payload`
> values between peers.
>
> Example: TCP byte stream, WebSocket message transport, or SHM transport.

> rs[term.link]
>
> A **link** is one concrete instantiation of a transport between two peers
> (`r[core.link]`).
>
> Example: one established TCP socket between peer A and peer B.

> rs[term.conduit]
>
> A **conduit** wraps a link and provides serialization/deserialization of payloads
> to and from postcard. This is the layer at which `StableConduit` implements seamless
> reconnection (allowing a session to hop to a new link).

> rs[term.session]
>
> A **session** allows multiplexing **connections** on a **conduit**. It is a
> state machines that sends and receives **messages**, keeping track of calls
> (also called requests), channels (created via calls), virtual connections
> (connections with ID>1), and flow control.

> rs[term.connection]
>
> Every session has an implit root virtual connection (`conn_id = 0`) and can
> carry additional virtual connections opened with `Connect`/`Accept`
> (`r[core.link.connection-zero]`, `r[core.conn.open]`).
>
> Each connection has its own namespace when it comes to request IDs and channel IDs.

> rs[term.virtual-connection]
> 
> Connections

> rs[term.channel]
>
> A **channel** is a directional data path identified by `(conn_id, channel_id)`.
> `Tx<T>` and `Rx<T>` are Rust-level handles over that mechanism
> (`r[channeling.type]`, `r[channeling.allocation.caller]`, `r[message.conn-id]`).
>
> Example: `Data { conn_id: 0, channel_id: 3, ... }` and
> `Data { conn_id: 7, channel_id: 3, ... }` refer to different channels.

## Trait Boundaries

> rs[layer.trait-boundaries]
>
> Each layer MUST correspond to one primary Rust trait boundary. Layer
> responsibilities MUST NOT leak across trait boundaries.

## Link

> rs[layer.transport]
>
> Concrete transport implementations are responsible only for moving roam
> `Payload` values and performing transport-specific framing/signaling.
>
> In Rust, these concrete transport implementations are exposed to the rest of
> the stack exclusively via the `Link` trait below (there is no separate
> higher-level "transport layer" beyond `Link`).

> rs[layer.link.interface]
>
> Rust MUST provide one shared **link** interface for all transports
> (stream, framed, shm), with a split sender/receiver model for typed `T`
> values encoded/decoded via `Codec`.

```rust
pub trait Link<T: 'static, C: Codec> {
    type Tx: LinkTx<T, C>;
    type Rx: LinkRx<T, C>;
    fn split(self) -> (Self::Tx, Self::Rx);
}

pub trait LinkTx<T, C: Codec>: Send + 'static {
    type Permit<'a>: LinkTxPermit<T, C>
    where
        Self: 'a;

    /// Wait for outbound capacity for exactly one item and reserve it for
    /// the caller.
    ///
    /// Cancellation of `reserve` MUST NOT leak capacity; if reservation
    /// succeeds, dropping the returned permit MUST return that capacity.
    async fn reserve(&self) -> io::Result<Self::Permit<'_>>;

    /// Request a graceful close of the outbound direction.
    ///
    /// This consumes `self` so it cannot be called twice.
    async fn close(self) -> io::Result<()>
    where
        Self: Sized;
}

pub trait LinkTxPermit<T, C: Codec> {
    /// Enqueue exactly one item into the reserved capacity.
    ///
    /// This MUST NOT block: backpressure happens at `reserve`, not at `send`.
    fn send(self, item: T) -> Result<(), C::EncodeError>;
}

pub trait LinkRx<T: 'static, C: Codec>: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;
    async fn recv(&mut self) -> Result<Option<SelfRef<T>>, Self::Error>;
}
```

> rs[layer.link.serialization]
>
> `Link` owns encoding/decoding between `T` and the underlying transport's
> bytes via a `Codec` `C`.
>
> `LinkRx::recv` yields `SelfRef<T>` (decoded value plus backing storage) so
> transports can support zero-copy borrows from their receive buffers.

## Codec

> rs[layer.codec.owner]
>
> Rust MUST define a dedicated **codec** layer above `Link` that owns
> serialization and deserialization between roam `Message` and `Payload`.
> For Rust, this serialization MUST use postcard.
>
> The codec layer exists to make the `Link` boundary mechanically enforceable:
> `Link` remains payload-opaque (`rs[layer.link.blind]`), while all knowledge of
> the `Message` type lives strictly above it.

> rs[layer.codec.interface]
>
> Rust MUST provide one primary codec trait boundary. The codec is plan-driven:
> it encodes/decodes any `T: Facet` using a precomputed `TypePlanCore`.
>
> The codec layer MUST be usable by transports to encode directly into their
> own buffers (stream write buffers, SHM slots, etc.) without intermediate
> staging.

```rust
pub enum EncodedPart<'a> {
    Skeleton(smallvec::SmallVec<[u8; 64]>),
    BorrowedBlob(&'a [u8]),
}

pub trait Codec: Send + Sync + 'static {
    type EncodeError: std::error::Error + Send + Sync + 'static;
    type DecodeError: std::error::Error + Send + Sync + 'static;

    fn encode_scatter<'a>(
        &self,
        value: facet_reflect::Peek<'a, 'a>,
        parts: &mut Vec<EncodedPart<'a>>,
    ) -> Result<usize, Self::EncodeError>;

    unsafe fn decode_into(
        &self,
        plan: &facet::TypePlanCore,
        bytes: &[u8],
        out: facet_core::PtrUninit,
    ) -> Result<(), Self::DecodeError>;
}
```

> rs[layer.codec.blind]
>
> The codec layer is protocol-blind above serialization:
> it MUST NOT implement request correlation, channel routing, flow control,
> reconnect/retry, virtual-connection logic, or any other roam semantics.
> Those behaviors belong to higher runtime layers.

> rs[layer.codec.errors]
>
> If deserialization fails during `LinkRx::recv`, `LinkRx::recv` MUST return
> `Err(...)` and the higher runtime layers MUST treat the link as failed (as
> with any other inbound framing error). The codec layer MUST NOT attempt
> recovery by skipping bytes or re-synchronizing within a link.

> rs[layer.link.permits]
>
> Link outbound buffering and backpressure MUST be permit-based:
>
> - `LinkTx::reserve` MUST await until outbound capacity for exactly one item
>   is available, then reserve it for the caller by returning a permit.
>   Capacity reservation MUST be FIFO-fair: if multiple tasks call `reserve`,
>   permits MUST be handed out in the order reservations were requested.
>   Cancelling a `reserve` call MUST forfeit the caller's place in that FIFO
>   order, but MUST NOT leak capacity.
>
> - `LinkTxPermit::send` MUST enqueue exactly one item into the reserved
>   capacity and MUST NOT block (beyond encoding).
>
> - Dropping a permit without calling `send` MUST release the reserved capacity
>   back to the link.
>
> - A link MUST have an internal outbound buffer (queue) with finite capacity.
>   This buffer is what permits reserve.
>
> Link receive and close behavior:
> - `LinkRx::recv` MUST await until one inbound item is available
>   (`Ok(Some(item))`), the inbound direction closes cleanly (`Ok(None)`), or
>   an I/O/framing/decode error occurs (`Err(...)`).
> - `LinkRx::recv` is single-consumer: it MUST NOT be awaited concurrently
>   by multiple tasks.
> - `LinkTx::close` MUST stop handing out new permits, MUST drain/flush all
>   already-enqueued outbound bytes, then MUST perform transport-level outbound
>   close (or return `Err(...)` if drain/flush/close fails).
> - `LinkTx::close` MUST NOT begin while there exist outstanding permits
>   derived from that sender. (In Rust, this is enforced by the permit borrowing
>   the sender and `close` consuming it.)
>   The purpose of `close` is to let higher layers shut down a link without
>   losing already-enqueued outbound bytes; it does not perform any roam-level
>   shutdown.
>
> Concurrency, ordering, and loss:
> - Link implementations MUST support concurrent send and receive on the same
>   link (one task reserving/sending while another receives).
> - Link implementations MUST preserve item ordering per direction.
> - Link implementations MUST NOT duplicate or silently drop enqueued items.

> rs[layer.link.blind]
>
> `Link` is protocol-blind: it MUST treat payload bytes as opaque and MUST NOT
> make decisions based on roam semantics (method IDs, request lifecycle, channel
> lifecycle, etc.). Transport-specific framing/signaling is allowed, but roam
> protocol behavior is not.

> rs[layer.link.connector]
>
> Rust MUST provide a transport-agnostic connector interface that yields fresh
> `Link` instances. Any component that needs to (re)establish links MUST depend
> on this interface.

```rust
pub trait TransportConnector {
    type Link: Link;
    async fn connect(&self) -> io::Result<Self::Link>;
}
```

> rs[layer.transport.stream]
>
> Stream transports (for example: TCP, Unix sockets) MUST use 4-byte
> little-endian length-prefix framing (`r[transport.bytestream.length-prefix]`).
> Each frame payload MUST carry exactly one roam `Payload`.
>
> Stream outbound queuing and draining entail:
> - `reserve` awaits queue capacity (in-process buffer).
> - `permit.send(payload)` enqueues one payload into that queue.
> - a stream writer task drains the queue in order:
>   - compute 4-byte little-endian length prefix for `payload.0.len()`
>   - write `len_prefix || payload_bytes` to the byte stream in order
>   - if the OS/socket cannot currently accept the full write, await until it can
>   - if an I/O error occurs, the link enters a failed state and subsequent
>     `reserve` MUST return `Err(...)`.
>
> Stream `recv` entails:
> - read exactly 4 bytes to obtain the length prefix
> - read exactly that many bytes into a new `Payload`
> - if the remote cleanly closes before a full prefix/payload is read, return
>   `Ok(None)` (end-of-stream)
> - if the stream ends mid-frame, return `Err(...)`
>
> Stream `close` entails:
> - flush any user-space buffered bytes for previously-enqueued payloads
> - then perform a half-close of the outbound stream direction (for example:
>   `shutdown(SHUT_WR)`), while leaving inbound receive possible until the peer
>   closes or an error occurs
>
> Stream buffering and backpressure entail:
> - buffering:
>   - in-process outbound queue (finite) is required by `rs[layer.link.permits]`
>   - OS socket send/receive buffers and user-space read/write buffers are also
>     part of the transport
> - backpressure: when the outbound queue is full, `reserve` MUST await; when
>   the OS socket send buffer is full, the writer task awaits; payloads MUST NOT
>   be dropped to relieve pressure.

> rs[layer.transport.framed]
>
> Framed transports (for example: WebSocket) MUST map one transport frame to
> one roam `Payload` (`r[transport.message.one-to-one]`).
>
> Framed outbound queuing and draining entail:
> - `reserve` awaits queue capacity (in-process buffer).
> - `permit.send(payload)` enqueues one payload into that queue.
> - a framed writer task drains the queue in order and submits exactly one
>   transport frame per payload, carrying exactly `payload.0` bytes.
> - if the framed sink reports an error, the link enters a failed state and
>   subsequent `reserve` MUST return `Err(...)`.
>
> Framed `recv` entails:
> - await exactly one inbound transport frame
> - return its bytes as one `Payload`
> - if the transport indicates a clean close, return `Ok(None)`
>
> Framed `close` entails:
> - flush any already-enqueued outbound frames
> - then perform the framed transport's graceful close procedure (for example:
>   WebSocket close handshake), without emitting any roam payloads on its own
>
> Framed buffering and backpressure entail:
> - buffering:
>   - in-process outbound queue (finite) is required by `rs[layer.link.permits]`
>   - frame queues and internal codec buffers are part of the framed transport
> - backpressure: when the outbound queue is full, `reserve` MUST await; when
>   the framed sink is not ready, the writer task awaits. `permit.send` MUST NOT
>   block.

> rs[layer.transport.shm]
>
> SHM transport MUST preserve the same link/virtual-connection semantics.
> Transport signaling/buffering is SHM-specific.
>
> SHM outbound queuing and draining entail:
> - `reserve` awaits queue capacity (in-process buffer).
> - `permit.send(payload)` enqueues one payload into that queue.
> - an SHM producer task drains the queue in order:
>   - copy payload bytes into exactly one outbound SHM slot/entry
>   - publish/signal availability to the peer
>   - if there is no SHM slot capacity, await until capacity is available
>   - if an SHM error occurs, the link enters a failed state and subsequent
>     `reserve` MUST return `Err(...)`.
>
> SHM `recv` entails:
> - await a signal indicating an inbound slot/entry is available
> - consume exactly one entry and copy its bytes into one `Payload`
> - if the peer has closed and no further entries remain, return `Ok(None)`
>
> SHM `close` entails:
> - publish a transport-level "producer closed" state for the outbound direction
> - wake any blocked receivers as required by the SHM transport design
> - ensure no additional outbound entries are published after close begins
>
> SHM buffering and backpressure entail:
> - buffering:
>   - in-process outbound queue (finite) is required by `rs[layer.link.permits]`
>   - bounded SHM rings/queues are also transport buffers; each entry is exactly
>     one payload
> - backpressure: when the outbound queue is full, `reserve` MUST await; when
>   the SHM ring/queue is full, the SHM producer task awaits. `permit.send` MUST
>   NOT block.

## Session

> rs[layer.session]
>
> The session layer owns all roam protocol state machine concerns over a live
> link:
> - Hello/HelloYourself handshake (limits negotiation, session resume, parity)
> - connection lifecycle (`Connect`/`Accept`/`Reject`/`Goodbye`)
> - request/response correlation by `(conn_id, request_id)`
> - channel message routing (`Data`/`Ack`/`Close`/`Reset`/`Credit`)
> - transparent resumption after link failure (exactly-once replay/dedup)
> - protocol violation handling and error mapping

> rs[layer.session.shutdown]
>
> Protocol-level shutdown behavior (including when/why to send `Goodbye`) is
> owned by the session layer, not by the transport/link abstraction.

> rs[layer.session.interface]
>
> Rust MUST provide one primary session interface that consumes a `Link`,
> exposes root-connection call capability, exposes incoming connection
> requests, and runs the receive/send loop.

```rust
pub trait Session {
    type IncomingConnections;

    fn root(&self) -> ConnectionHandle;
    fn incoming(&mut self) -> &mut Self::IncomingConnections;
    fn run(self) -> impl Future<Output = Result<(), SessionError>>;
}
```

## Flow Control

> rs[layer.flow-control.owner]
>
> Channel flow control is owned by the session layer, not by generated clients
> and not by method-specific call code.

> rs[layer.flow-control.rules]
>
> Flow control semantics MUST follow main-spec rules
> (`r[flow.channel.credit-based]`, `r[flow.channel.all-transports]`,
> `r[flow.channel.credit-grant]`, `r[flow.channel.credit-consume]`,
> `r[flow.channel.zero-credit]`) for all transports. Transport-specific
> mechanics are allowed:
> - stream/framed: wire `Credit` messages
> - shm: shared-memory credit counters
>
> These differences MUST remain observationally equivalent at the protocol
> level.

> rs[layer.flow-control.interface]
>
> Rust MUST define a dedicated flow-control trait used by the session layer.

```rust
pub trait FlowControl {
    fn on_data_received(&mut self, channel_id: ChannelId, bytes: u32);
    fn wait_for_send_credit(
        &mut self,
        channel_id: ChannelId,
        bytes: u32,
    ) -> impl Future<Output = io::Result<()>>;
    fn consume_send_credit(&mut self, channel_id: ChannelId, bytes: u32);
}
```

## Call Runtime

> rs[layer.call-runtime]
>
> The call runtime owns the typed-to-message boundary:
> - argument encoding into Request payload (`r[call.request.payload-encoding]`)
> - schema-driven channel ID collection for Request.channels
>   (`r[call.request.channels]`, `r[call.request.channels.schema-driven]`)
> - response decoding (`r[call.response.encoding]`)
> - response-side `Rx<T>` binding

> rs[layer.call-runtime.interface]
>
> Rust MUST define one primary call-runtime interface for typed method calls.

> rs[layer.call-runtime.single-codepath]
>
> Rust MUST implement exactly one call/dispatch codepath for every method,
> always using the channel-aware request representation. The "simple" case is
> `channels.len() == 0`.

## Generated Clients

> rs[layer.generated-client]
>
> Generated Rust service clients are concrete typed API structs with method
> impls. They own method descriptors, typed argument construction, and one
> inner call-runtime handle; they MUST NOT own link I/O, reconnect/retry
> control flow, or channel protocol mechanics.

> rs[client.nongeneric]
>
> Generated Rust service clients MUST expose non-generic concrete types.
> Runtime polymorphism, when needed, is an internal runtime concern.

## What a Call Entails (Rust)

> rs[call.steps]
>
> A Rust call path MUST perform:
> 1. Encode typed args as Request payload tuple
> 2. Collect all channel IDs from args using schema-driven traversal
> 3. Send Request with `method_id`, `metadata`, `channels`, and `payload`
> 4. Correlate Response by `request_id`
> 5. Decode `Result<T, RoamError<E>>` from response payload
> 6. Bind response `Rx<T>` handles using response channels + method schema

## Reconnection and Retry

> rs[reconnect.owner]
>
> Automatic reconnection and session resume are owned by session
> implementations (not by generated service clients).

> rs[reconnect.new-link]
>
> Reconnection creates a new link instance. It does not resume the previous
> link instance.

> rs[reconnect.resume]
>
> After a link failure, a reconnecting session implementation MUST establish a
> new link and MUST attempt to resume the existing session using the
> Hello/HelloYourself handshake as specified by the main spec.
>
> If resumption succeeds, the session MUST preserve all connection IDs and all
> request/channel ID spaces, and MUST complete in-flight calls/channels
> exactly-once using `CallAck` and channel `Ack` rules from the main spec.
>
> If resumption fails, the session MUST surface a terminal session-lost error
> to user-visible handles (calls and channels) that depended on the old
> session.

> rs[reconnect.trigger]
>
> Reconnect/retry behavior MUST follow reconnect-spec rules:
> - trigger on transport failures (`r[reconnect.trigger.transport]`)
> - do NOT trigger on RPC-level errors (`r[reconnect.trigger.not-rpc]`)
> - apply retry policy/backoff (`r[reconnect.policy]`, `r[reconnect.policy.backoff]`)
> - preserve shared/single-reconnect concurrency semantics
>   (`r[reconnect.concurrency.shared]`, `r[reconnect.concurrency.single-reconnect]`)

> rs[retry.semantic]
>
> Retry means re-issuing the same logical call after reconnect due to transport
> failure. This is transport-failure recovery behavior, not application-level
> retry policy.

> rs[reconnect.inflight.requests]
>
> In-flight requests on a failed link MUST complete with transport failure
> unless explicitly retried by reconnect logic.

> rs[reconnect.inflight.channels]
>
> Channels on a failed link MUST be resumed and completed exactly-once if the
> session is resumed successfully. If the session cannot be resumed, channels
> MUST fail with a session-lost error.

## Channel Binding

> rs[channel.binding.request]
>
> Request-side channel ownership is caller-side and schema-driven
> (`r[channeling.allocation.caller]`, `r[call.request.channels.schema-driven]`).

> rs[channel.binding.server]
>
> On dispatch, Rust MUST treat Request.channels as authoritative and patch IDs
> into deserialized args before binding streams (`r[channeling.allocation.caller]`).

> rs[channel.binding.response]
>
> On the caller side, Rust MUST bind response `Rx<T>` handles after decoding
> a successful response, using response channel IDs and method schema.

> rs[channel.binding.transport-agnostic]
>
> Channel binding semantics are transport-agnostic and MUST be identical across
> framed, stream, and shm transports.

# Method Identity

Every method has a unique 64-bit identifier computed from its service name,
method name, and signature.

> rs[method.identity.computation]
>
> The method ID MUST be computed as:
> ```
> method_id = blake3(kebab(ServiceName) + "." + kebab(methodName) + sig_bytes)[0..8]
> ```
> Where:
> - `kebab()` converts to kebab-case (e.g. `TemplateHost` → `template-host`)
> - `sig_bytes` is the BLAKE3 hash of the method's argument and return types
> - `[0..8]` takes the first 8 bytes as a u64

This means:
- Renaming a service or method changes the ID (breaking change)
- Changing the signature changes the ID (breaking change)
- Case variations normalize to the same ID (`loadTemplate` = `load_template`)

# Signature Hash

The `sig_bytes` used in method identity is a BLAKE3 hash of the method's
structural signature. This is computed at compile time by the `#[roam::service]`
macro using [facet](https://facet.rs) type introspection.

> rs[signature.hash.algorithm]
>
> The signature hash MUST be computed by hashing a canonical byte
> representation of the method signature using BLAKE3.

> rs[signature.varint]
>
> Variable-length integers (`varint`) in signature encoding use the
> same format as [POSTCARD]: unsigned LEB128. Each byte contains 7
> data bits; the high bit indicates continuation (1 = more bytes).

> rs[signature.endianness]
>
> All fixed-width integers in signature encoding are little-endian.
> The final u64 method ID is extracted as the first 8 bytes of the
> BLAKE3 hash, interpreted as little-endian.

The canonical representation encodes the method signature as a tuple (see
`rs[signature.method]` below). Each type within is encoded recursively:

## Primitive Types

> rs[signature.primitive]
>
> Primitive types MUST be encoded as a single byte tag:

| Type | Tag |
|------|-----|
| `bool` | `0x01` |
| `u8` | `0x02` |
| `u16` | `0x03` |
| `u32` | `0x04` |
| `u64` | `0x05` |
| `u128` | `0x06` |
| `i8` | `0x07` |
| `i16` | `0x08` |
| `i32` | `0x09` |
| `i64` | `0x0A` |
| `i128` | `0x0B` |
| `f32` | `0x0C` |
| `f64` | `0x0D` |
| `char` | `0x0E` |
| `String` | `0x0F` |
| `()` (unit) | `0x10` |
| `bytes` | `0x11` |

## Container Types

> rs[signature.container]
>
> Container types MUST be encoded as a tag byte followed by their element type(s):

| Type | Tag | Encoding |
|------|-----|----------|
| List | `0x20` | tag + encode(element) |
| Option | `0x21` | tag + encode(inner) |
| Array | `0x22` | tag + varint(len) + encode(element) |
| Map | `0x23` | tag + encode(key) + encode(value) |
| Set | `0x24` | tag + encode(element) |
| Tuple | `0x25` | tag + varint(len) + encode(T1) + encode(T2) + ... |
| Stream | `0x26` | tag + encode(element) |

Note: These are wire-format types, not Rust types. `Vec`, `VecDeque`, and
`LinkedList` all encode as List. `HashMap` and `BTreeMap` both encode as Map.

> rs[signature.bytes.equivalence]
>
> Any "bytes" type MUST use the `bytes` tag (`0x11`) in signature encoding.
> This includes the dedicated `bytes` wire-format type and a list of `u8`.
> As a result, `bytes` and `List<u8>` MUST produce identical signature hashes.

## Struct Types

> rs[signature.struct]
>
> Struct types MUST be encoded as:
> ```
> 0x30 + varint(field_count) + (field_name + field_type)*
> ```
> Where each `field_name` is encoded as `varint(len) + utf8_bytes`.
> Fields MUST be encoded in declaration order.

Note: The struct's *name* is NOT included — only field names and types.
This allows renaming types without breaking compatibility.

## Enum Types

> rs[signature.enum]
>
> Enum types MUST be encoded as:
> ```
> 0x31 + varint(variant_count) + (variant_name + variant_payload)*
> ```
> Where each `variant_name` is encoded as `varint(len) + utf8_bytes`.
> `variant_payload` is:
> - `0x00` for unit variants
> - `0x01` + encode(T) for newtype variants
> - `0x02` + struct encoding (without the 0x30 tag) for struct variants

Variants MUST be encoded in declaration order.

## Recursive Types

> rs[signature.recursive]
>
> When encoding types that reference themselves (directly or indirectly),
> implementations MUST detect cycles and emit a back-reference tag (`0x32`)
> instead of infinitely recursing. Cycles can occur through any chain of
> type references: containers, struct fields, enum variants, or combinations
> thereof.
>
> The back-reference tag is a single byte that indicates "this type was
> already encoded earlier in this signature". This ensures:
> - No stack overflow during encoding
> - Deterministic output (same type always produces same bytes)
> - Finite signature size for recursive types

## Method Signature Encoding

> rs[signature.method]
>
> A method signature MUST be encoded as a tuple of its arguments followed
> by the return type:
> ```
> 0x25 + varint(arg_count) + encode(arg1) + ... + encode(argN) + encode(return_type)
> ```

This structure ensures unambiguous parsing — without the argument count,
`fn add(a: i32, b: i32) -> i64` would have the same bytes as
`fn foo(a: i32, b: i32, c: i64)` (which returns unit).

## Example

For a method:
```rust
async fn add(&self, a: i32, b: i32) -> i64;
```

The canonical bytes would be:
```
0x25          // Tuple tag (method signature)
0x02          // 2 arguments
0x09          // a: i32
0x09          // b: i32
0x0A          // return: i64
```

BLAKE3 hash of these bytes gives `sig_bytes`.

# SHM Introduction

This document specifies the shared memory (SHM) hub transport binding for
roam. The hub topology supports one **host** and multiple **guests** (1:N),
designed for plugin systems where a host application loads guest plugins
that communicate via shared memory.

> shm[shm.scope]
>
> This binding encodes Core Semantics over shared memory. It does NOT
> redefine the meaning of calls, channels, errors, or flow control —
> only their representation in this transport.

> shm[shm.architecture]
>
> This binding assumes:
> - All processes sharing the segment run on the **same architecture**
>   (same endianness, same word size, same atomic semantics)
> - Cross-process atomics are valid (typically true on modern OSes)
> - The shared memory region is cache-coherent
>
> Cross-architecture SHM is not supported.

# Topology

## Hub (1:N)

> shm[shm.topology.hub]
>
> The hub topology has exactly one **host** and zero or more **guests**.
> The host creates and owns the shared memory segment. Guests attach to
> communicate with the host.

```
         ┌─────────┐
         │  Host   │
         └────┬────┘
              │
    ┌─────────┼─────────┐
    │         │         │
┌───┴───┐ ┌───┴───┐ ┌───┴───┐
│Guest 1│ │Guest 2│ │Guest 3│
└───────┘ └───────┘ └───────┘
```

> shm[shm.topology.hub.communication]
>
> Guests communicate only with the host, not with each other. Each
> guest has its own BipBuffers and channel table within the shared
> segment, and all guests share the VarSlotPool.

> shm[shm.topology.hub.calls]
>
> Either the host or a guest can initiate calls. The host can call
> methods on any guest; a guest can call methods on the host.

## Peer Identification

> shm[shm.topology.peer-id]
>
> A guest's `peer_id` (u8) is 1 + the index of its entry in the peer
> table. Peer table entry 0 corresponds to `peer_id = 1`, entry 1 to
> `peer_id = 2`, etc. The host does not have a peer_id (it is not in
> the peer table).

> shm[shm.topology.max-guests]
>
> The maximum number of guests is limited to 255 (peer IDs 1-255).
> The `max_guests` field in the segment header MUST be ≤ 255. The
> peer table has exactly `max_guests` entries.

## ID Widths

Core defines `request_id` and `channel_id` as u64. SHM uses narrower
encodings to fit in the 24-byte frame header:

> shm[shm.id.request-id]
>
> SHM encodes `request_id` as u32. The upper 32 bits of Core's u64
> `request_id` are implicitly zero. Implementations MUST NOT use
> request IDs ≥ 2^32.

> shm[shm.id.channel-id]
>
> SHM encodes `channel_id` as u32. The upper 32 bits of Core's u64
> `channel_id` are implicitly zero.

## Channel ID Allocation

> shm[shm.id.channel-scope]
>
> Channel IDs are scoped to the guest-host pair. Two different guests
> may independently use the same `channel_id` value without collision
> because they have separate channel tables.

> shm[shm.id.channel-parity]
>
> Within a guest-host pair, channel IDs use odd/even parity to prevent
> collisions:
> - The **host** allocates even channel IDs (2, 4, 6, ...)
> - The **guest** allocates odd channel IDs (1, 3, 5, ...)
>
> Channel ID 0 is reserved and MUST NOT be used.

## Request ID Scope

> shm[shm.id.request-scope]
>
> Request IDs are scoped to the guest-host pair. Two different guests
> may use the same `request_id` value without collision because their
> BipBuffers are separate.

# Handshake

Core Semantics require a Hello exchange to negotiate connection parameters.
SHM replaces this with the segment header:

> shm[shm.handshake]
>
> SHM does not use Hello messages. Instead, the segment header fields
> (`max_payload_size`, `initial_credit`, `max_channels`) serve as the
> host's unilateral configuration. Guests accept these values by
> attaching to the segment.

> shm[shm.handshake.no-negotiation]
>
> Unlike networked transports, SHM has no negotiation — the host's
> values are authoritative. A guest that cannot operate within these
> limits MUST NOT attach.

# Segment Layout

The host creates a shared memory segment containing all communication
state for all guests.

## Segment Header

> shm[shm.segment.header]
>
> The segment MUST begin with a header:

```
Offset  Size   Field                Description
──────  ────   ─────                ───────────
0       8      magic                Magic bytes: "RAPAHUB\x01"
8       4      version              Segment format version (2)
12      4      header_size          Size of this header (128)
16      8      total_size           Initial segment size in bytes
24      4      max_payload_size     Maximum payload per message
28      4      initial_credit       Initial channel credit (bytes)
32      4      max_guests           Maximum number of guests (≤ 255)
36      4      bipbuf_capacity      BipBuffer data region size in bytes per direction
40      8      peer_table_offset    Offset to peer table
48      8      slot_region_offset   (legacy, unused in v2)
56      4      slot_size            Must be 0 in v2 (fixed pools eliminated)
60      4      inline_threshold     Max frame size for inline payloads (0 = default 256)
64      4      max_channels         Max concurrent channels per guest
68      4      host_goodbye         Host goodbye flag (0 = active)
72      8      heartbeat_interval   Heartbeat interval in nanoseconds (0 = disabled)
80      8      var_slot_pool_offset Offset to shared VarSlotPool (must be non-zero in v2)
88      8      current_size         Current segment size (≥ total_size if extents appended)
96      32     reserved             Reserved for future use (zero)
```

> shm[shm.segment.header-size]
>
> The segment header is 128 bytes.

> shm[shm.segment.magic]
>
> The magic field MUST be exactly `RAPAHUB\x01` (8 bytes).

## Peer Table

> shm[shm.segment.peer-table]
>
> The peer table contains one entry per potential guest:

```rust
#[repr(C)]
struct PeerEntry {
    state: AtomicU32,           // 0=Empty, 1=Attached, 2=Goodbye, 3=Reserved
    epoch: AtomicU32,           // Incremented on attach
    _reserved_head_tail: [AtomicU32; 4], // Reserved (v1 ring indices, zeroed in v2)
    last_heartbeat: AtomicU64,  // Monotonic tick count (see shm[shm.crash.heartbeat-clock])
    ring_offset: u64,           // Offset to this guest's area (BipBuffers)
    slot_pool_offset: u64,      // Reserved in v2 (0)
    channel_table_offset: u64,  // Offset to this guest's channel table
    reserved: [u8; 8],          // Reserved (zero)
}
// Total: 64 bytes per entry
```
>
> In v2, the four ring head/tail fields are reserved (zeroed). Ring state
> (write, read, watermark) lives in the BipBuffer headers within the
> guest area instead of in the peer table.

> shm[shm.segment.peer-state]
>
> Peer states:
> - **Empty (0)**: Slot available for a new guest
> - **Attached (1)**: Guest is active
> - **Goodbye (2)**: Guest is shutting down or has crashed
> - **Reserved (3)**: Host has allocated slot, guest not yet attached (see `shm[shm.spawn.reserved-state]`)

## Per-Guest BipBuffers

Each guest has two BipBuffers (variable-length byte SPSC ring buffers):
- **Guest→Host BipBuffer**: Guest produces frames, host consumes
- **Host→Guest BipBuffer**: Host produces frames, guest consumes

> shm[shm.bipbuf.layout]
>
> Each guest's area (at `PeerEntry.ring_offset`) contains:
> 1. Guest→Host BipBuffer: 128-byte header + `bipbuf_capacity` bytes data
> 2. Host→Guest BipBuffer: 128-byte header + `bipbuf_capacity` bytes data
> 3. Channel table: `max_channels × 16` bytes
>
> Total per guest: `align64(2 × (128 + bipbuf_capacity)) + align64(max_channels × 16)`.

> shm[shm.bipbuf.header]
>
> Each BipBuffer has a 128-byte header (two cache lines):
>
> ```rust
> #[repr(C, align(64))]
> struct BipBufHeader {
>     // --- Cache line 0: producer-owned ---
>     write: AtomicU32,       // Next write position (byte offset)
>     watermark: AtomicU32,   // Wrap boundary (0 = no wrap active)
>     capacity: u32,          // Data region size in bytes (immutable)
>     _pad0: [u8; 52],
>
>     // --- Cache line 1: consumer-owned ---
>     read: AtomicU32,        // Consumed frontier (byte offset)
>     _pad1: [u8; 60],
> }
> ```
>
> Splitting producer and consumer fields onto separate cache lines avoids
> false sharing.

> shm[shm.bipbuf.initialization]
>
> On segment creation, all BipBuffer memory MUST be zeroed (write=0,
> watermark=0, read=0). On guest attach, the guest MUST NOT assume buffer
> contents are valid.

> shm[shm.bipbuf.grant]
>
> To reserve `n` contiguous bytes for writing (**grant**):
>
> 1. If `write >= read`:
>    - If `capacity - write >= n`: grant `[write..write+n)`. Done.
>    - Else if `read > 0`: set `watermark = write`, `write = 0`. If
>      `n < read`, grant `[0..n)`. Else undo (`write = old`, `watermark = 0`)
>      and return full.
>    - Else (`read == 0`): no room to wrap, return full.
> 2. If `write < read`:
>    - If `write + n < read`: grant `[write..write+n)`. Done.
>    - Else return full.

> shm[shm.bipbuf.commit]
>
> After writing data into a granted region, the producer commits by
> advancing `write += n` with **Release** ordering. This makes the
> written bytes visible to the consumer.

> shm[shm.bipbuf.read]
>
> To read available bytes:
>
> 1. Load `watermark` with **Acquire**.
> 2. If `watermark != 0` (wrap active):
>    - If `read < watermark`: readable region is `[read..watermark)`.
>    - If `read >= watermark`: set `read = 0`, `watermark = 0`, retry.
> 3. If `watermark == 0`, load `write` with **Acquire**:
>    - If `read < write`: readable region is `[read..write)`.
>    - Otherwise the buffer is empty.

> shm[shm.bipbuf.release]
>
> After processing `n` bytes, the consumer releases by advancing
> `read += n` with **Release** ordering. If `read` reaches or exceeds
> `watermark`, set `read = 0` and `watermark = 0` (wrap to beginning).

> shm[shm.bipbuf.full]
>
> If the BipBuffer has no room for the requested grant, the producer
> MUST wait. Implementations SHOULD use a doorbell signal or polling
> with backoff. A full BipBuffer indicates backpressure from a slow
> consumer, not a protocol error.

> shm[shm.backpressure.host-to-guest]
>
> When the host cannot write to a guest's H→G BipBuffer (buffer full
> or no slots available), it MUST queue the message and retry when
> capacity becomes available. The guest signals the doorbell after
> consuming messages, which the host uses as a cue to drain its
> pending send queue.

# Message Encoding

All abstract messages from Core are encoded as variable-length frames
written into BipBuffers.

## ShmFrameHeader (24 bytes)

> shm[shm.frame.header]
>
> Each frame begins with a 24-byte header (little-endian):
>
> ```text
> Offset  Size  Field        Description
> ──────  ────  ─────        ───────────
> 0       4     total_len    Frame size in bytes (including this field), padded to 4
> 4       1     msg_type     Message type (see shm[shm.desc.msg-type])
> 5       1     flags        Bit 0: FLAG_SLOT_REF (payload in VarSlotPool)
> 6       2     _reserved    Reserved (zero)
> 8       4     id           request_id or channel_id (u32)
> 12      8     method_id    Method hash (u64, 0 for non-Request messages)
> 20      4     payload_len  Actual payload byte count
> ```

> shm[shm.frame.alignment]
>
> `total_len` MUST be padded up to a 4-byte boundary. The padding
> bytes (between the end of the payload or slot reference and the next
> 4-byte boundary) SHOULD be zeroed.

> shm[shm.frame.inline]
>
> When `FLAG_SLOT_REF` is clear (flags bit 0 = 0), the payload bytes
> immediately follow the 24-byte header inline in the BipBuffer.
> The frame occupies `align4(24 + payload_len)` bytes total.

> shm[shm.frame.slot-ref]
>
> When `FLAG_SLOT_REF` is set (flags bit 0 = 1), a 12-byte `SlotRef`
> follows the header instead of inline payload:
>
> ```text
> Offset  Size  Field            Description
> ──────  ────  ─────            ───────────
> 0       1     class_idx        Size class index in VarSlotPool
> 1       1     extent_idx       Extent index within the class
> 2       2     _pad             Reserved (zero)
> 4       4     slot_idx         Slot index within the extent
> 8       4     slot_generation  ABA generation counter
> ```
>
> The actual payload is stored in the VarSlotPool slot identified by
> this reference. The frame occupies 36 bytes (`align4(24 + 12)`).

> shm[shm.frame.threshold]
>
> A frame with `24 + payload_len <= inline_threshold` SHOULD be sent
> inline. Larger payloads MUST use a slot reference. The default
> inline threshold is 256 bytes (configurable via the segment header's
> `inline_threshold` field; 0 means use the default).

## Metadata Encoding

The abstract Message type (see [CORE-SPEC]) has separate `metadata` and
`payload` fields. SHM frames carry both in the payload region, combined
into a single encoded value:

> shm[shm.metadata.in-payload]
>
> For Request and Response messages, the frame's payload contains both
> metadata and arguments/result, encoded as a single [POSTCARD] value:
>
> ```rust
> struct RequestPayload {
>     metadata: Vec<(String, MetadataValue)>,
>     arguments: T,  // method arguments tuple
> }
>
> struct ResponsePayload {
>     metadata: Vec<(String, MetadataValue)>,
>     result: Result<T, RoamError<E>>,
> }
> ```
>
> This differs from other transports where metadata and payload are
> separate fields in the Message enum.

> shm[shm.metadata.limits]
>
> The limits from `shm[call.metadata.limits]` apply: at most 128 keys,
> each value at most 16 KB. Violations are connection errors.

## Message Types

> shm[shm.desc.msg-type]
>
> The `msg_type` field in `ShmFrameHeader` identifies the abstract message:
>
> | Value | Message | `id` Field Contains |
> |-------|---------|---------------------|
> | 1 | Request | `request_id` |
> | 2 | Response | `request_id` |
> | 3 | Cancel | `request_id` |
> | 4 | Data | `channel_id` |
> | 5 | Close | `channel_id` |
> | 6 | Reset | `channel_id` |
> | 7 | Goodbye | (unused) |
> | 8 | Connect | `request_id` |
> | 9 | Accept | `request_id` |
> | 10 | Reject | `request_id` |

> shm[shm.flow.no-credit-message]
>
> There is no Credit message type. Credit is conveyed via shared
> atomic counters in the channel table (see [Flow Control](#flow-control)).

## Payload Encoding

> shm[shm.payload.encoding]
>
> Payloads MUST be [POSTCARD]-encoded.

> shm[shm.payload.inline]
>
> If `24 + payload_len <= inline_threshold`, the payload SHOULD be
> stored inline in the BipBuffer frame (see `shm[shm.frame.inline]`).
> The default inline threshold is 256 bytes.

> shm[shm.payload.slot]
>
> If `24 + payload_len > inline_threshold`, the payload MUST be stored
> in the shared VarSlotPool and referenced via a `SlotRef` in the frame
> (see `shm[shm.frame.slot-ref]`).

## Slot Lifecycle

> shm[shm.slot.allocate]
>
> To allocate a slot from the VarSlotPool:
> 1. Find the smallest size class where `slot_size >= payload_len`
> 2. Pop from that class's Treiber stack free list (see `shm[shm.varslot.allocation]`)
> 3. If exhausted, try the next larger class
> 4. Increment the slot's generation counter
> 5. Write payload to the slot's data region

> shm[shm.slot.free]
>
> To free a slot:
> 1. Push it back onto the size class's Treiber stack free list
>    (see `shm[shm.varslot.freeing]`)
>
> The receiver frees slots after processing the message.

> shm[shm.slot.exhaustion]
>
> If no free slots are available in any suitable size class, the sender
> MUST wait. Use polling with backoff. Slot exhaustion is not a protocol
> error — it indicates backpressure.

# Ordering and Synchronization

## Memory Ordering

> shm[shm.ordering.ring-publish]
>
> When writing a frame to a BipBuffer:
> 1. Grant contiguous space (check write vs read positions)
> 2. Write frame header and payload into the granted region
> 3. Commit: advance `write += total_len` with **Release** ordering

> shm[shm.ordering.ring-consume]
>
> When reading frames from a BipBuffer:
> 1. Load `write` with **Acquire** ordering
> 2. If readable bytes available, read frame(s)
> 3. Process message(s)
> 4. Release: advance `read += bytes_consumed` with **Release** ordering

## Wakeup Mechanism

Use doorbells for efficient cross-process notification, complemented by
polling with backoff:

> shm[shm.wakeup.consumer-wait]
>
> **Consumer waiting for messages** (BipBuffer empty):
> - Wait: poll on doorbell fd (or busy-wait with backoff)
> - Wake: Producer signals doorbell after committing bytes

> shm[shm.wakeup.producer-wait]
>
> **Producer waiting for space** (BipBuffer full):
> - Wait: poll on doorbell fd (or busy-wait with backoff)
> - Wake: Consumer signals doorbell after releasing bytes

> shm[shm.wakeup.credit-wait]
>
> **Sender waiting for credit** (zero remaining):
> - Wait: futex_wait on `ChannelEntry.granted_total`
> - Wake: Receiver calls futex_wake on `granted_total` after updating

> shm[shm.wakeup.slot-wait]
>
> **Sender waiting for slots** (VarSlotPool exhausted):
> - Wait: poll with backoff (implementation-defined strategy)
> - Wake: Receiver signals after freeing a slot back to the pool

> shm[shm.wakeup.fallback]
>
> On non-Linux platforms, use polling with exponential backoff or
> platform-specific primitives (e.g., `WaitOnAddress` on Windows).

# Flow Control

SHM uses shared counters for flow control instead of explicit Credit
messages.

## Channel Metadata Table

> shm[shm.flow.channel-table]
>
> Each guest-host pair has a **channel metadata table** for tracking
> active channels. The table is located at a fixed offset within the
> guest's region:

```rust
#[repr(C)]
struct ChannelEntry {
    state: AtomicU32,        // 0=Free, 1=Active, 2=Closed
    granted_total: AtomicU32, // Cumulative bytes authorized
    _reserved: [u8; 8],      // Reserved (zero)
}
// 16 bytes per entry
```

> shm[shm.flow.channel-table-location]
>
> Each guest's channel table offset is stored in `PeerEntry.channel_table_offset`.
> The table size is `max_channels * 16` bytes.

> shm[shm.flow.channel-table-indexing]
>
> The `channel_id` directly indexes the channel table: channel N uses
> entry N. This means:
> - Channel IDs MUST be < `max_channels`
> - Channel ID 0 is reserved; entry 0 is unused
> - Usable channel IDs are 1 to `max_channels - 1`

> shm[shm.flow.channel-activate]
>
> When opening a new channel, the allocator MUST initialize the entry:
> 1. Set `granted_total = initial_credit` (from segment header)
> 2. Set `state = Active` (with Release ordering)
>
> The sender maintains its own `sent_total` counter locally (not in
> shared memory).

> shm[shm.flow.channel-id-reuse]
>
> A channel ID MAY be reused after the channel is closed (Close or Reset
> received by both peers). To reuse:
> 1. Sender sends Close or Reset
> 2. Receiver sets `ChannelEntry.state = Free` (with Release ordering)
> 3. Allocator polls for `state == Free` before reusing
>
> On reuse, the allocator reinitializes per `shm[shm.flow.channel-activate]`.
>
> Implementations SHOULD delay reuse to avoid races (e.g., wait for
> the entry to be Free before reallocating).

## Credit Counters

> shm[shm.flow.counter-per-channel]
>
> Each active channel has a `granted_total: AtomicU32` counter in its
> channel table entry. The receiver publishes; the sender reads.

## Counter Semantics

> shm[shm.flow.granted-total]
>
> `granted_total` is cumulative bytes authorized by the receiver.
> Monotonically increasing (modulo wrap).

> shm[shm.flow.remaining-credit]
>
> remaining = `granted_total - sent_total` (wrapping subtraction).
> Sender MUST NOT send if remaining < payload size.

> shm[shm.flow.wrap-rule]
>
> Interpret `granted_total - sent_total` as signed i32. Negative or
> > 2^31 indicates corruption.

## Memory Ordering for Credit

> shm[shm.flow.ordering.receiver]
>
> Update `granted_total` with Release after consuming data.

> shm[shm.flow.ordering.sender]
>
> Load `granted_total` with Acquire before deciding to send.

## Initial Credit

> shm[shm.flow.initial]
>
> Channels start with `granted_total = initial_credit` from segment
> header. Sender's `sent_total` starts at 0.

## Zero Credit

> shm[shm.flow.zero-credit]
>
> Sender waits. Use futex on the counter to avoid busy-wait.
> Receiver wakes after granting credit.

## Credit and Reset

> shm[shm.flow.reset]
>
> After Reset, stop accessing the channel's credit counter. Values
> after Reset are undefined.

# Guest Lifecycle

## Attaching

> shm[shm.guest.attach]
>
> To attach, a guest:
> 1. Opens the shared memory segment
> 2. Validates magic and version
> 3. Finds an Empty peer table entry
> 4. Atomically sets state from Empty to Attached (CAS)
> 5. Increments epoch
> 6. Begins processing

> shm[shm.guest.attach-failure]
>
> If no Empty slots exist, the guest cannot attach (hub is full).

## Detaching

> shm[shm.guest.detach]
>
> To detach gracefully:
> 1. Set state to Goodbye
> 2. Drain remaining messages
> 3. Complete or cancel in-flight work
> 4. Unmap segment

## Host Observing Guests

> shm[shm.host.poll-peers]
>
> The host periodically checks peer states. On observing Goodbye or
> epoch change (crash), the host cleans up that guest's resources.

# Failure and Goodbye

## Goodbye

> shm[shm.goodbye.guest]
>
> A guest signals shutdown by setting its peer state to Goodbye.
> It MAY send a Goodbye frame with reason first.

> shm[shm.goodbye.host]
>
> The host signals shutdown by setting `host_goodbye` in the header
> to a non-zero value. Guests MUST poll this field and detach when
> it becomes non-zero.

> shm[shm.goodbye.payload]
>
> A Goodbye frame's payload is a [POSTCARD]-encoded `String`
> containing the reason. Per `shm[core.error.goodbye-reason]`, the
> reason MUST contain the rule ID that was violated.

> shm[shm.goodbye.host-atomic]
>
> The `host_goodbye` field MUST be accessed atomically (load/store
> with at least Relaxed ordering). It is written by the host and
> read by all guests.

## Crash Detection

The host is responsible for detecting crashed guests. Epoch-based
detection only works when a new guest attaches; the host needs
additional mechanisms to detect a guest that crashed while attached.

> shm[shm.crash.host-owned]
>
> The host MUST use an out-of-band mechanism to detect crashed guests.
> Common approaches:
> - Hold a process handle (e.g., `pidfd` on Linux, process handle on
>   Windows) and detect termination
> - Require guests to update a heartbeat field periodically
> - Use OS-specific death notifications

> shm[shm.crash.heartbeat]
>
> If using heartbeats: each `PeerEntry` contains a `last_heartbeat:
> AtomicU64` field. Guests MUST update this at least every
> `heartbeat_interval` nanoseconds (from segment header). The host
> declares a guest crashed if heartbeat is stale by more than
> `2 * heartbeat_interval`.

> shm[shm.crash.heartbeat-clock]
>
> Heartbeat values are **monotonic clock readings**, not wall-clock time.
> All processes read from the same system monotonic clock, so values are
> directly comparable without synchronization.
>
> Each process writes its current monotonic clock reading (in nanoseconds)
> to `last_heartbeat`. The host compares the guest's value against its
> own clock reading: if `host_now - guest_heartbeat > 2 * heartbeat_interval`,
> the guest is considered crashed.
>
> Platform clock sources:
> - Linux: `CLOCK_MONOTONIC` (via `clock_gettime` or `Instant`)
> - Windows: `QueryPerformanceCounter`
> - macOS: `mach_absolute_time`

> shm[shm.crash.epoch]
>
> Guests increment epoch on attach. If epoch changes unexpectedly,
> the previous instance crashed and was replaced.

> shm[shm.crash.recovery]
>
> On detecting a crashed guest, the host MUST:
> 1. Set the peer state to Goodbye
> 2. Treat all in-flight operations as failed
> 3. Reset BipBuffer headers (write=0, read=0, watermark=0) for both
>    the G→H and H→G buffers
> 4. Scan VarSlotPool for slots owned by the crashed peer
>    (`owner_peer == peer_id`) and return them to their free lists
> 5. Reset channel table entries to Free
> 6. Set state to Empty (allowing new guest to attach)

# Byte Accounting

> shm[shm.bytes.what-counts]
>
> For flow control, "bytes" = `payload_len` of Data frames (the
> [POSTCARD]-encoded element size). Frame header overhead and slot
> padding do NOT count.

# File-Backed Segments

For cross-process communication, the SHM segment must be backed by a
file that can be memory-mapped by multiple processes.

## Segment File

> shm[shm.file.path]
>
> The host creates the segment as a regular file at a path known to
> both host and guests. Common locations:
> - `/dev/shm/<name>` (Linux tmpfs, recommended)
> - `/tmp/<name>` (portable but may be disk-backed)
> - Application-specific directory
>
> The path MUST be communicated to guests out-of-band (e.g., via
> command-line argument or environment variable).

> shm[shm.file.create]
>
> To create a segment file:
> 1. Open or create the file with read/write permissions
> 2. Truncate to the required `total_size`
> 3. Memory-map the entire file with `MAP_SHARED`
> 4. Initialize all data structures (header, peer table, BipBuffers,
>    VarSlotPool, channel tables)
> 5. Write header magic last (signals segment is ready)

> shm[shm.file.attach]
>
> To attach to an existing segment:
> 1. Open the file read/write
> 2. Memory-map with `MAP_SHARED`
> 3. Validate magic and version
> 4. Read configuration from header
> 5. Proceed with guest attachment per `shm[shm.guest.attach]`

> shm[shm.file.permissions]
>
> The segment file SHOULD have permissions that allow all intended
> guests to read and write. On POSIX systems, mode 0600 or 0660 is
> typical, with the host and guests running as the same user or group.

> shm[shm.file.cleanup]
>
> The host SHOULD delete the segment file on graceful shutdown. On
> crash, stale segment files may remain; implementations SHOULD handle
> this (e.g., by deleting and recreating on startup).

## Platform Mapping

> shm[shm.file.mmap-posix]
>
> On POSIX systems, use `mmap()` with:
> - `PROT_READ | PROT_WRITE`
> - `MAP_SHARED` (required for cross-process visibility)
> - File descriptor from `open()` or `shm_open()`

> shm[shm.file.mmap-windows]
>
> On Windows, use:
> - `CreateFileMapping()` to create a file mapping object
> - `MapViewOfFile()` to map it into the process address space
> - Named mappings can use `Global\<name>` for cross-session access

# Peer Spawning

The host typically spawns guest processes and provides them with the
information needed to attach to the segment.

## Spawn Ticket

> shm[shm.spawn.ticket]
>
> Before spawning a guest, the host:
> 1. Allocates a peer table entry (finds Empty slot, sets to Reserved)
> 2. Creates a doorbell pair (see Doorbell section)
> 3. Prepares a "spawn ticket" containing:
>    - `hub_path`: Path to the segment file
>    - `peer_id`: Assigned peer ID (1-255)
>    - `doorbell_fd`: Guest's end of the doorbell (Unix only)

> shm[shm.spawn.reserved-state]
>
> The peer entry state during spawning:
> - Host sets state to **Reserved** before spawn
> - Guest sets state to **Attached** after successful attach
> - If spawn fails, host resets state to **Empty**
>
> The Reserved state prevents other guests from claiming the slot.

```rust
#[repr(u32)]
pub enum PeerState {
    Empty = 0,
    Attached = 1,
    Goodbye = 2,
    Reserved = 3,  // Host has allocated, guest not yet attached
}
```

## Command-Line Arguments

> shm[shm.spawn.args]
>
> The canonical way to pass spawn ticket information to a guest process
> is via command-line arguments:
>
> ```
> --hub-path=<path>    # Path to segment file
> --peer-id=<id>       # Assigned peer ID (1-255)
> --doorbell-fd=<fd>   # Doorbell file descriptor (Unix only)
> ```

> shm[shm.spawn.fd-inheritance]
>
> On Unix, the doorbell file descriptor MUST be inheritable by the child
> process. The host MUST NOT set `O_CLOEXEC` / `FD_CLOEXEC` on the
> guest's doorbell fd before spawning. After spawn, the host closes its
> copy of the guest's doorbell fd (keeping only its own end).

## Guest Initialization

> shm[shm.spawn.guest-init]
>
> A spawned guest process:
> 1. Parses command-line arguments to extract ticket info
> 2. Opens and maps the segment file
> 3. Validates segment header
> 4. Locates its peer entry using `peer_id`
> 5. Verifies state is Reserved (set by host)
> 6. Atomically sets state from Reserved to Attached
> 7. Initializes doorbell from the inherited fd
> 8. Begins message processing

# Doorbell Mechanism

Doorbells provide instant cross-process wakeup and death detection,
complementing BipBuffer-based communication.

## Purpose

> shm[shm.doorbell.purpose]
>
> A doorbell is a bidirectional notification channel between host and
> guest that provides:
> - **Wakeup**: Signal the other side to check for work
> - **Death detection**: Detect when the other process terminates
>
> Unlike futex (which requires polling shared memory), a doorbell
> allows blocking on I/O that unblocks immediately when the peer dies.

## Implementation

> shm[shm.doorbell.socketpair]
>
> On Unix, doorbells are implemented using `socketpair()`:
>
> ```c
> int fds[2];
> socketpair(AF_UNIX, SOCK_STREAM, 0, fds);
> // fds[0] = host end, fds[1] = guest end
> ```
>
> The host keeps `fds[0]` and passes `fds[1]` to the guest via the
> spawn ticket.

> shm[shm.doorbell.signal]
>
> To signal the peer, write a single byte to the socket:
>
> ```c
> char byte = 1;
> write(doorbell_fd, &byte, 1);
> ```
>
> The byte value is ignored; only the wakeup matters.

> shm[shm.doorbell.wait]
>
> To wait for a signal (with optional timeout):
>
> ```c
> struct pollfd pfd = { .fd = doorbell_fd, .events = POLLIN };
> poll(&pfd, 1, timeout_ms);
> if (pfd.revents & POLLIN) {
>     // Peer signaled - check for work
>     char buf[16];
>     read(doorbell_fd, buf, sizeof(buf));  // drain
> }
> if (pfd.revents & (POLLHUP | POLLERR)) {
>     // Peer died
> }
> ```

> shm[shm.doorbell.death]
>
> When a process terminates, its end of the socketpair is closed by the
> kernel. The surviving process sees `POLLHUP` or `POLLERR` on its end,
> providing immediate death notification without polling.

## Integration with BipBuffers

> shm[shm.doorbell.ring-integration]
>
> Doorbells complement BipBuffer-based messaging:
> - After committing a frame to the BipBuffer, signal the doorbell
> - The receiver can `poll()` both the doorbell fd and other I/O
> - On doorbell signal, check BipBuffers for new frames
>
> This avoids busy-waiting and integrates with async I/O frameworks.

> shm[shm.doorbell.optional]
>
> Doorbell support is OPTIONAL. Implementations MAY use only futex-based
> wakeup (per `shm[shm.wakeup.*]`). Doorbells are recommended when:
> - Death detection latency is critical
> - Integration with async I/O (epoll/kqueue/IOCP) is desired
> - Busy-waiting must be avoided entirely

# Death Notification

The host needs to detect when guest processes crash or hang so it can
clean up resources and optionally restart them.

## Notification Callback

> shm[shm.death.callback]
>
> When adding a peer, the host MAY register a death callback:
>
> ```rust
> type DeathCallback = Arc<dyn Fn(PeerId) + Send + Sync>;
>
> struct AddPeerOptions {
>     peer_name: Option<String>,
>     on_death: Option<DeathCallback>,
> }
> ```
>
> The callback is invoked when the guest's doorbell indicates death
> (POLLHUP/POLLERR) or when heartbeat timeout is exceeded.

> shm[shm.death.callback-context]
>
> The death callback:
> - Is called from the host's I/O or monitor thread
> - Receives the `peer_id` of the dead guest
> - SHOULD NOT block for long (schedule cleanup asynchronously)
> - MAY trigger guest restart logic

## Detection Methods

> shm[shm.death.detection-methods]
>
> Implementations SHOULD use multiple detection methods:
>
> | Method | Latency | Reliability | Platform |
> |--------|---------|-------------|----------|
> | Doorbell POLLHUP | Immediate | High | Unix |
> | Heartbeat timeout | 2× interval | Medium | All |
> | Process handle | Immediate | High | All |
> | Epoch change | On reattach | Low | All |
>
> Doorbell provides the best latency on Unix. Process handles (pidfd
> on Linux, process handle on Windows) provide immediate notification
> on all platforms.

> shm[shm.death.process-handle]
>
> On Linux 5.3+, use `pidfd_open()` to get a pollable fd for the child
> process. On Windows, the process handle from `CreateProcess()` is
> waitable. This provides kernel-level death notification without
> relying on doorbells.

## Recovery Actions

> shm[shm.death.recovery]
>
> On guest death detection, per `shm[shm.crash.recovery]`:
> 1. Invoke the death callback (if registered)
> 2. Set peer state to Goodbye, then Empty
> 3. Reset BipBuffers and reclaim VarSlotPool slots
> 4. Close host's doorbell end
> 5. Optionally respawn the guest

# Variable-Size Slot Pools

In v2, the shared VarSlotPool is the **only** slot mechanism. All
payloads that exceed the inline threshold are stored in the VarSlotPool.
Fixed-size per-guest bitmap pools (v1) have been eliminated.

## Size Classes

> shm[shm.varslot.classes]
>
> A variable-size pool consists of multiple **size classes**, each with
> its own slot size and count. Example configuration:
>
> | Class | Slot Size | Count | Total | Use Case |
> |-------|-----------|-------|-------|----------|
> | 0 | 1 KB | 1024 | 1 MB | Small RPC args |
> | 1 | 16 KB | 256 | 4 MB | Typical payloads |
> | 2 | 256 KB | 32 | 8 MB | Images, CSS |
> | 3 | 4 MB | 8 | 32 MB | Compressed fonts |
> | 4 | 16 MB | 4 | 64 MB | Decompressed fonts |
>
> The specific configuration is application-dependent.

> shm[shm.varslot.selection]
>
> To allocate a slot for a payload of size `N`:
> 1. Find the smallest size class where `slot_size >= N`
> 2. Allocate from that class's free list
> 3. If exhausted, try the next larger class (optional)
> 4. If all classes exhausted, block or return error

## Shared Pool Architecture

> shm[shm.varslot.shared]
>
> The VarSlotPool is **shared** across all guests:
> - One pool region for the entire hub (at `var_slot_pool_offset`)
> - All guests and the host allocate from the same size classes
> - Slot ownership is tracked per-allocation
>
> This allows efficient use of memory when different guests have
> different payload size distributions.

> shm[shm.varslot.ownership]
>
> Each slot tracks its current owner:
>
> ```rust
> struct SlotMeta {
>     generation: AtomicU32,  // ABA counter
>     state: AtomicU32,       // Free=0, Allocated=1, InFlight=2
>     owner_peer: AtomicU32,  // Peer ID that allocated (0 = host)
>     next_free: AtomicU32,   // Free list link
> }
> ```
>
> When a guest crashes, slots with `owner_peer == crashed_peer_id`
> are returned to their respective free lists.

## Extent-Based Growth

> shm[shm.varslot.extents]
>
> Size classes can grow dynamically via **extents**:
> - Each size class starts with one extent of `slot_count` slots
> - When exhausted, additional extents can be allocated
> - Extents are appended to the segment file (requires remap)
>
> ```rust
> struct SizeClassHeader {
>     slot_size: u32,
>     slots_per_extent: u32,
>     extent_count: AtomicU32,
>     extent_offsets: [AtomicU64; MAX_EXTENTS],
> }
> ```

> shm[shm.varslot.extent-layout]
>
> Each extent contains:
> 1. Extent header (class, index, slot count, offsets)
> 2. Slot metadata array (one `SlotMeta` per slot)
> 3. Slot data array (actual payload storage)
>
> ```
> ┌─────────────────────────────────────────────────┐
> │ ExtentHeader (64 bytes)                         │
> ├─────────────────────────────────────────────────┤
> │ SlotMeta[0] │ SlotMeta[1] │ ... │ SlotMeta[N-1] │
> ├─────────────────────────────────────────────────┤
> │ Slot[0] data │ Slot[1] data │ ... │ Slot[N-1]   │
> └─────────────────────────────────────────────────┘
> ```

## Free List Management

> shm[shm.varslot.freelist]
>
> Each size class maintains a lock-free free list using a Treiber stack:
>
> ```rust
> struct SizeClassHeader {
>     // ...
>     free_head: AtomicU64,  // Packed (index, generation)
> }
> ```
>
> Allocation pops from the head; freeing pushes to the head. The
> generation counter prevents ABA problems.

> shm[shm.varslot.allocation]
>
> To allocate from a size class:
> 1. Load `free_head` with Acquire
> 2. If empty (sentinel value), class is exhausted
> 3. Load the slot's `next_free` pointer
> 4. CAS `free_head` from current to next
> 5. On success, increment slot's generation, set state to Allocated
> 6. On failure, retry from step 1

> shm[shm.varslot.freeing]
>
> To free a slot:
> 1. Verify generation matches (detect double-free)
> 2. Set slot state to Free
> 3. Load current `free_head`
> 4. Set slot's `next_free` to current head
> 5. CAS `free_head` to point to this slot
> 6. On failure, retry from step 3

# References

- **[CORE-SPEC]** roam Core Specification
  <@/spec/_index.md#core-semantics>

- **[POSTCARD]** Postcard Wire Format Specification
  <https://postcard.jamesmunns.com/wire-format>
