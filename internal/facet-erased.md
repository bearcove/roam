# Facet Opaque Adapter for Erased Payloads

## Purpose

Define the contract behind `#[facet(opaque = thing)]` so Roam can:

1. serialize erased payloads on send,
2. defer payload deserialization on receive,
3. keep low-copy behavior when backing is stable.

This document is the API contract, not an implementation sketch.

## Problem

Roam envelopes can be parsed before payload concrete type is known.

- Send path needs: erased value -> concrete bytes.
- Receive path needs: raw payload bytes -> stored deferred payload.

Facet must let opaque types provide this behavior explicitly.

## Annotation

```rust
#[facet(opaque = PayloadAdapter)]
pub enum Payload<'payload> {
    Borrowed { /* type-erased outgoing value */ },
    Raw { /* deferred incoming bytes */ },
}
```

`PayloadAdapter` is a type that implements the adapter trait below.

## Public Adapter Interface

The public interface is typed. No raw pointer parameters are exposed here.

```rust
pub struct OpaqueSerialize {
    pub ptr: PtrConst,
    pub shape: &'static Shape,
    pub plan: Option<&'static TypePlanCore>,
}

pub enum OpaqueDeserialize {
    Borrowed { offset: usize, len: usize },
    Owned(Vec<u8>),
}

pub trait FacetOpaqueAdapter<T> {
    type Error;

    /// Map a concrete opaque value to erased serialization inputs.
    fn serialize_map(value: &T) -> OpaqueSerialize;

    /// Build the concrete opaque value from deferred payload input.
    fn deserialize_build(input: OpaqueDeserialize) -> Result<T, Self::Error>;
}
```

## Semantics

### Send path

1. Serializer reaches a `#[facet(opaque = ...)]` field.
2. It calls `FacetOpaqueAdapter::<T>::serialize_map(&value)`.
3. Returned `(ptr, shape, plan)` defines what to serialize.
4. Format serializes that value using normal format rules.

### Receive path

1. Format decodes payload bytes as a byte sequence.
2. If bytes are in a stable backing buffer, pass:
   `OpaqueDeserialize::Borrowed { offset, len }`.
3. Otherwise pass:
   `OpaqueDeserialize::Owned(Vec<u8>)`.
4. Call `FacetOpaqueAdapter::<T>::deserialize_build(...)`.
5. Store returned `T` in the destination field.

## Borrowed vs Owned Rules

1. `Borrowed { offset, len }` means the range is inside the format's current
   input backing and remains valid for the opaque value's required lifetime.
2. If that guarantee cannot be made, the format MUST use `Owned(Vec<u8>)`.
3. Adapter decides internal storage layout for both modes.

## Internal Lowering

Facet derive may lower the typed trait methods to raw-pointer function pointers
in shape metadata. That lowering is internal only.

- Public API stays typed (`&T`, `Result<T, _>`, enum input).
- Internal vtable may use pointer-based shims for uniform dispatch.

## Roam Outcome

This contract enables both directions cleanly:

1. Send: erased payloads serialize without ad-hoc manual encoding.
2. Receive: envelope parses first, payload bytes are retained as deferred raw,
   then decoded later with concrete method/item type.
3. Low-copy path is preserved via `Borrowed { offset, len }` when possible.

## Non-goals

1. Facet does not impose one universal payload storage type.
2. Facet does not require all formats to support borrowed input.
3. Facet does not encode transport/backing ownership policy in this trait.
