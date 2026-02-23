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
| `Raw`      | Write `backing[offset..offset+len]` verbatim (forward) | Read length prefix, store backing + range   |

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
    Value { ptr: PtrConst, shape: &'static Shape },

    /// Pre-serialized bytes — the serializer writes them verbatim
    /// (with whatever framing the format requires).
    /// Used by `Payload::Raw` for forwarding/proxying.
    Bytes { bytes: &'a [u8] },
}
```

For deserialization, the format handles all I/O (reads the length
prefix, determines the byte range). It then needs to tell the opaque
type "here's your byte range" so the type can store it. This is done
through a second vtable function:

```rust
/// The format has parsed the opaque region and determined the byte
/// range. Initialize the opaque value at self_ptr to store this
/// range. The opaque type decides how to represent it internally
/// (e.g. Payload::Raw { backing, offset, len }).
///
/// `input_ptr` and `input_len` point into the backing's buffer.
/// The format passes these after reading its own framing (e.g.
/// varint length prefix for postcard).
type OpaqueDeserializeStoreFn = unsafe fn(
    self_ptr: PtrUninit,
    input_ptr: *const u8,
    input_len: usize,
) -> Result<PtrMut, /* error */ ()>;
```

The `Backing` ownership is handled separately — the `SelfRef<Message>`
already keeps the backing alive, so the `Payload::Raw` variant just
needs to record the pointer + length into that backing. No ownership
transfer at the vtable level.

The format drives the whole process:
1. Format hits the opaque field during deserialization
2. Format reads its framing (varint length prefix)
3. Format calls the vtable with the byte range
4. Opaque type stores the range internally

### TypePlanCore on Payload

`Payload::Borrowed` should also carry an `Option<&'static TypePlanCore>`
for the inner value's type. This isn't needed for serialization (the
`Peek` + `Shape` suffice), but will be useful for future optimizations
where the serializer can use a precomputed plan instead of walking the
shape graph.

## Receive path: deferred deserialization (raw bytes)

### What happens today

On the receive side, the conduit deserializes `Frame<Message>`. But the
dispatch layer needs to inspect `Message.method_id` to determine the
concrete args type — which is only possible *after* the message
envelope is deserialized. We can't deserialize the `Payload` field
eagerly because we don't know what type it should be.

### What we need

A "raw value" concept — facet-postcard's equivalent of serde's
`RawValue`. When the deserializer encounters a `Payload` field, it
should:

1. Note the current byte offset in the input
2. Skip over the encoded value without interpreting it (walk the
   postcard encoding to find where the value ends)
3. Record the byte range `(start_offset, end_offset)`
4. Store this range in the `Payload` so the dispatch layer can
   deserialize it later with the correct type

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

### Skip-and-record in postcard

Postcard is a TLV-ish format. Skipping a value requires walking the
shape to know how many bytes to consume (varints are variable-length,
sequences have length prefixes, etc.). But we don't need the *value's*
shape — we need a shape-unaware skip.

Actually, we **do** need the shape. Postcard doesn't have self-
describing framing. A `u32` is encoded as a varint (1-5 bytes), a
`struct { a: u32, b: u32 }` is two varints back-to-back with no
delimiter. Without knowing the type, we can't know how many bytes to
skip.

This means the "skip" must be driven by *something*. Options:

**Option A: Length-prefix the payload.** Before serializing the inner
value, write a varint length prefix. On deserialize, read the length,
skip that many bytes, record the range. This adds a few bytes per
message but makes skipping trivial and shape-independent.

**Option B: Shape-driven skip.** The deserializer uses the inner
value's `Shape` (which must be known to the receive side somehow —
perhaps from the `method_id` → type mapping registered at startup) to
walk the value without constructing it. But this means we need the
type info *before* we can skip, which defeats the purpose of deferring.

**Option A is strongly preferred.** The length prefix is tiny (1-5
bytes for messages up to 4GB), and it makes the receive path clean:
read length → slice the bytes → done. No shape lookup needed during
the initial frame parse.

### Integration with the send path

If we go with Option A (length-prefixed payload), the send path needs
to know the payload's serialized size before writing it. This is
exactly what the scatter plan provides — after building the scatter
plan for the inner value, we know `plan.total_size`, write it as a
varint, then write the plan's segments. The `Payload`'s vtable hook
would signal "serialize inner value with length prefix."

## Summary of facet changes needed

1. **Opaque-inner vtable hook** (`facet-core`): a way for opaque types
   to say "serialize this `(ptr, shape)` in my place." Used on the
   send path.

2. **Raw-value deserialization** (`facet-postcard`): when deserializing
   a field marked for deferred parsing, read a length prefix and store
   the byte range without interpreting the contents. Used on the
   receive path.

3. **Scatter/gather API** (`facet-postcard`, issue #2065): produces a
   scatter plan instead of writing to a `Vec`. Required for both
   the send path (zero-copy into write slots) and the length-prefix
   calculation (need to know payload size before writing the prefix).
