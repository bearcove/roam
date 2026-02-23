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
