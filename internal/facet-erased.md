# Erased serialization and deferred deserialization in facet

## Problem

Roam's wire type is `Frame<Message<'payload>>`. The `Message` enum
contains `Payload` fields in `Request.args`, `Response.payload`, and
`ChannelItem.payload`. `Payload` is `#[facet(opaque)]` and carries a
type-erased value:

```rust
#[facet(opaque)]
pub enum Payload<'payload> {
    Borrowed {
        ptr: PtrConst,
        shape: &'static Shape,
        _phantom2: PhantomData<&'payload ()>,
    },
}
```

When facet-postcard serializes `Frame<Message>`, it hits the `Payload`
field and sees an opaque type. It doesn't know how to serialize it
because the actual value (the RPC method arguments, response type, or
channel item) is behind a type-erased pointer.

There are two distinct problems — one for send, one for receive.

## Send path: serialize through a type-erased value

### What happens today

The serializer calls `serialize_opaque_scalar` when it encounters
`Payload`. This hook receives a `Peek` over the `Payload` enum itself,
not over the inner value. The serializer has no way to reach through to
the actual `(ptr, shape)` and serialize that value.

### What we need

When the serializer encounters an opaque type that wraps a type-erased
value, it should be able to **recurse into the inner value's shape**.
The opaque type provides `(PtrConst, &'static Shape)` — exactly the
ingredients for `Peek::unchecked_new`. The serializer constructs a
`Peek` from these and continues serialization as if the inner value
were directly in the struct.

### Design: erased serialization vtable

`Payload` is an enum with variant-specific serialization behavior. The
facet annotation needs to tell serializers how to handle it — not as a
single vtable hook, but as a **format-level dispatch** that routes
differently depending on the variant:

| Variant    | Serialize                                              | Deserialize                                 |
|------------|--------------------------------------------------------|---------------------------------------------|
| `Borrowed` | Construct `Peek` from `(ptr, shape)`, recurse          | N/A (never deserialized into)               |
| `Raw`      | Write `backing[offset..offset+len]` verbatim (forward) | Deserialize bytes, store offset + len       |

The vtable provides **data, not I/O**. The serializer/deserializer
does all reading and writing. The vtable just tells the format what
to serialize or where to store deserialized results.

```rust
/// Given a pointer to the opaque value, return what the serializer
/// should serialize in its place.
type OpaqueContentFn = unsafe fn(PtrConst) -> OpaqueContent;

enum OpaqueContent<'a> {
    /// Type-erased value — the serializer constructs a Peek from
    /// (ptr, shape) and recurses into it. The format decides its
    /// own framing (e.g. postcard writes a varint length prefix
    /// before the inner value's bytes).
    /// Used by `Payload::Borrowed` on the send path.
    Value {
        ptr: PtrConst,
        shape: &'static Shape,
        /// Optional precomputed plan for plan-driven serialization.
        /// When present, the serializer can skip shape-graph walking.
        plan: Option<&'static TypePlanCore>,
    },

    /// Pre-serialized bytes — the serializer writes them verbatim
    /// (with whatever framing the format requires).
    /// Used by `Payload::Raw` for forwarding/proxying.
    Bytes { bytes: &'a [u8] },
}
```

For deserialization, the key insight is that **from the format's
perspective, `Payload` looks like `&[u8]`**. Each format already knows
how to serialize and deserialize byte sequences using its own native
encoding. Postcard uses a varint length prefix followed by raw bytes.
JSON would use base64. The format doesn't need special instructions —
it just sees bytes.

The vtable's deserialize side tells the opaque type "here are your
bytes" after the format has already done all the I/O:

```rust
/// The format has deserialized a byte sequence for this opaque
/// field. Initialize the opaque value at self_ptr to store the
/// byte range. The opaque type decides how to represent it
/// internally (e.g. Payload::Raw { backing, offset, len }).
///
/// `offset` and `len` describe the byte range within the input
/// buffer. The format passes these after deserializing the byte
/// sequence using its own native encoding.
type OpaqueDeserializeStoreFn = unsafe fn(
    self_ptr: PtrUninit,
    offset: usize,
    len: usize,
) -> Result<PtrMut, /* error */ ()>;
```

The `Backing` ownership is handled separately — the `SelfRef<Message>`
already keeps the backing alive, so the `Payload::Raw` variant just
needs to record the offset + length into that backing. No ownership
transfer at the vtable level.

The format drives the whole process:
1. Format hits the opaque field during deserialization
2. Format deserializes a byte sequence using its native encoding
   (e.g. postcard reads a varint length, then that many raw bytes)
3. Format calls the vtable with the byte range (offset + len)
4. Opaque type stores the range internally

## Receive path: deferred deserialization (raw bytes)

### What happens today

On the receive side, the conduit deserializes `Frame<Message>`. But the
dispatch layer needs to inspect `Message.method_id` to determine the
concrete args type — which is only possible *after* the message
envelope is deserialized. We can't deserialize the `Payload` field
eagerly because we don't know what type it should be.

### What we need

A "raw value" concept — facet-postcard's equivalent of serde's
`RawValue`. Since `Payload` presents as `&[u8]` to the format, the
deserializer handles it the way it handles any byte sequence:

1. Read the byte sequence using the format's native encoding
   (for postcard: read varint length, then that many raw bytes)
2. Record the byte range `(offset, len)` within the input buffer
3. Call the vtable to store this range in the `Payload` so the
   dispatch layer can deserialize it later with the correct type

On the receive side, `Payload` would have a `Bytes` variant (or
similar) holding a reference into the backing:

```rust
pub enum Payload<'payload> {
    // Send path: type-erased borrowed value, serialize through (ptr, shape)
    Borrowed {
        ptr: PtrConst,
        shape: &'static Shape,
        _phantom: PhantomData<&'payload ()>,
    },
    // Receive path: owned raw bytes, deserialize later with known type
    Raw {
        backing: Backing,
        offset: usize,
        len: usize,
    },
}
```

The `Raw` variant owns its `Backing` — the handle that keeps the
underlying bytes alive (heap `Box<[u8]>`, BipBuf region, VarSlot,
mmap). Sent messages borrow from the caller's scope; received messages
are owned (fully allocated or refcounted buffers that may return to a
pool). The `'payload` lifetime is irrelevant for `Raw` — it carries
no borrows.

The dispatch layer then:
1. Reads `message.method_id`
2. Looks up the handler → gets the concrete `Shape` + `TypePlanCore`
3. Deserializes `backing[offset..offset+len]` into the concrete args
   type, producing a `SelfRef<T>` that keeps the backing alive

### Why this works without special framing

Postcard is not self-describing — a `u32` is a varint (1-5 bytes), a
struct is fields back-to-back with no delimiters. Without knowing the
type, you can't skip over a value. This seems like it would make
deferred deserialization impossible.

But since `Payload` presents as `&[u8]` to the format, postcard
encodes it the same way it encodes any byte sequence: **varint length
prefix + raw bytes**. On the receive side, postcard reads the varint
length, knows exactly how many bytes to consume, and passes the range
to the vtable. No shape knowledge needed during the initial frame
parse.

On the send side, the vtable's `OpaqueContentFn` returns
`OpaqueContent::Value { ptr, shape, plan }`. The serializer serializes
the inner value into bytes (using the shape/plan), and writes those
bytes the same way it writes any `&[u8]` — with whatever framing the
format uses natively. For postcard, that's a varint length prefix.
The scatter plan provides the total size, so the length prefix can be
computed before the payload bytes are written.

## Summary of facet changes needed

1. **Opaque-content vtable** (`facet-core`): `OpaqueContentFn` and
   `OpaqueDeserializeStoreFn` on the Shape for opaque types. The
   serialize side returns `OpaqueContent` (either a type-erased value
   or pre-serialized bytes). The deserialize side accepts a byte range
   (offset + len). The format treats the opaque field as `&[u8]` and
   uses its native byte-sequence encoding.

2. **Opaque-aware serialization** (`facet-postcard` and other formats):
   when serializing an opaque type, call the vtable to get content,
   serialize it as bytes. When deserializing, deserialize a byte
   sequence and call the vtable to store the range. No special framing
   — the format's existing byte-sequence encoding handles everything.

3. **Scatter/gather API** (`facet-postcard`, issue #2065): produces a
   scatter plan instead of writing to a `Vec`. Required for the send
   path (zero-copy into write slots) and for computing the payload
   size before writing the format's native length encoding.
