# Facet Opaque Adapter for Roam Payload

## Purpose

Define the contract behind `#[facet(opaque = thing)]` so Roam can:

1. serialize erased outgoing payloads,
2. parse incoming envelopes before payload concrete type is known,
3. defer payload decoding with low-copy behavior when possible.

## Core Invariant

For incoming data, the runtime object is `SelfRef<Message>`: `(Backing, Message)`.

`Backing` is single-owner and not split into sub-backings. Therefore deferred
payload state inside `Message` stores either:

1. a borrowed byte slice into the message backing, or
2. owned bytes.

It does not store a second `Backing` handle for a payload subrange.

## Annotation

```rust
#[facet(opaque = PayloadAdapter)]
pub enum Payload<'payload> {
    Borrowed { /* outgoing erased value */ },
    RawBorrowed(&'payload [u8]),
    RawOwned(Vec<u8>),
}
```

`PayloadAdapter` is a type that implements the adapter trait below.

## Public Adapter Interface

Public interface is typed. Pointer shims are internal.

```rust
pub struct OpaqueSerialize {
    pub ptr: PtrConst,
    pub shape: &'static Shape,
    pub plan: Option<&'static TypePlanCore>,
}

pub enum OpaqueDeserialize<'de> {
    Borrowed(&'de [u8]),
    Owned(Vec<u8>),
}

pub trait FacetOpaqueAdapter {
    type Error;
    type SendValue;
    type RecvValue<'de>;

    /// Outgoing path: map typed value to erased serialization inputs.
    fn serialize_map(value: &Self::SendValue) -> OpaqueSerialize;

    /// Incoming path: build deferred payload representation.
    fn deserialize_build<'de>(input: OpaqueDeserialize<'de>)
        -> Result<Self::RecvValue<'de>, Self::Error>;
}
```

## Directional Semantics

### Send

1. Serializer reaches `#[facet(opaque = ...)]` field.
2. Calls `serialize_map(...)`.
3. Uses returned `(ptr, shape, plan)` to serialize payload bytes.

### Receive

1. Parser decodes payload bytes.
2. If parser input can borrow stably, call `deserialize_build(Borrowed(&[u8]))`.
3. Otherwise call `deserialize_build(Owned(Vec<u8>))`.
4. Store returned deferred payload value inside `Message`.

## Where Slicing Lives

Slicing belongs to parser/input logic, not to payload storage types.

1. Parser decides whether borrowed slice is valid.
2. Adapter boundary receives either borrowed slice or owned bytes.
3. If parser internally tracks ranges, it resolves range -> slice before calling
   `deserialize_build`.

## Roam Dispatch Transform

Incoming flow is:

1. Parse envelope into `SelfRef<Message>`.
2. Read `method_id` and resolve concrete args `(Shape, TypePlanCore)`.
3. Consume/map `SelfRef<Message>` into `SelfRef<ConcreteArgs>` using the same
   backing.

This is a move/transform, not a backing split.

## Outgoing vs Incoming

1. Incoming messages live in `SelfRef<Message>` and can use `RawBorrowed(&[u8])`.
2. Outgoing messages are not in `SelfRef<Message>` and use erased outgoing form
   (`Borrowed { ... }`) or owned raw bytes.

## Non-goals

1. No universal facet-wide backing container.
2. No requirement that every format support borrowed input.
3. No payload-level ownership model that splits or clones transport `Backing`.
