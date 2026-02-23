+++
title = "roam signature specification"
description = "Method identity and signature hash computation"
weight = 11
+++

# Method Identity

Every method has a unique 64-bit identifier computed from its service name,
method name, and signature.

> r[method.identity.computation]
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

> r[signature.hash.algorithm]
>
> The signature hash MUST be computed by hashing a canonical byte
> representation of the method signature using BLAKE3.

> r[signature.varint]
>
> Variable-length integers (`varint`) in signature encoding use the
> same format as [POSTCARD]: unsigned LEB128. Each byte contains 7
> data bits; the high bit indicates continuation (1 = more bytes).

> r[signature.endianness]
>
> All fixed-width integers in signature encoding are little-endian.
> The final u64 method ID is extracted as the first 8 bytes of the
> BLAKE3 hash, interpreted as little-endian.

The canonical representation encodes the method signature as a tuple (see
`r[signature.method]` below). Each type within is encoded recursively:

## Primitive Types

> r[signature.primitive]
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

> r[signature.container]
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

> r[signature.bytes.equivalence]
>
> Any "bytes" type MUST use the `bytes` tag (`0x11`) in signature encoding.
> This includes the dedicated `bytes` wire-format type and a list of `u8`.
> As a result, `bytes` and `List<u8>` MUST produce identical signature hashes.

## Struct Types

> r[signature.struct]
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

> r[signature.enum]
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

> r[signature.recursive]
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

> r[signature.method]
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
