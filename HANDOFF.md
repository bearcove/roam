# Handoff: Replacing TypeDetail with facet::Shape

## Context

We're working on GitHub issue #2: removing the custom `TypeDetail` type system in favor of using `facet::Shape` directly.

**Why**: `TypeDetail` is a less precise copy of facet's `Shape`. Since facet already provides complete type introspection via `Shape`, maintaining a parallel type system is unnecessary overhead.

## What's Been Done

### 1. Core Infrastructure ✅

- **`roam-schema`**: Added `ShapeKind`, `StructInfo`, `EnumInfo`, `VariantKind` types and `classify_shape()` / `classify_variant()` functions. These provide the abstraction layer for codegen to work with Shape.
- **`roam-hash`**: Fully migrated to use `Shape`, `ScalarType`, and the facet-core API.
- **`roam-codegen/render.rs`**: Switched from custom `kebab()` to `heck::ToKebabCase`.

### 2. Codegen Targets - Partial Progress

| Target     | Status | Notes |
|------------|--------|-------|
| TypeScript | ✅ Done | Fully migrated |
| Java       | ✅ Done | Fully migrated |
| Python     | ✅ Done | Fully migrated |
| **Rust**   | ❌ TODO | **PRIORITY** - Start here |
| Go         | ❌ TODO | |
| Swift      | ❌ TODO | |

## What Needs To Be Done

### 1. **PRIORITY: Rust Codegen** (`rust/roam-codegen/src/targets/rust.rs`)

This is the most important target since it's the primary language for roam.

**Current errors:**
```
error[E0432]: unresolved imports `roam_schema::TypeDetail`, `roam_schema::VariantDetail`, `roam_schema::VariantPayload`
```

**Pattern to follow** (from TypeScript migration):

1. Update imports:
   ```rust
   use facet_core::{ScalarType, Shape, StructKind};
   use roam_schema::{
       EnumInfo, MethodDetail, ServiceDetail, ShapeKind, StructInfo, VariantKind,
       classify_shape, classify_variant, is_bytes, is_rx, is_tx,
   };
   ```

2. Change function signatures from `&TypeDetail` to `&'static Shape`

3. Replace `match ty { TypeDetail::X => ... }` with `match classify_shape(shape) { ShapeKind::X => ... }`

4. Replace `match &v.payload { VariantPayload::X => ... }` with `match classify_variant(v) { VariantKind::X => ... }`

5. Replace `arg.type_info` with `arg.ty` and `method.return_type` stays the same (it's already `&'static Shape`)

**Key functions to update:**
- `rust_type()` - convert Shape to Rust type string
- `generate_encode_expr()` / `generate_decode_stmt()` - codec generation
- `collect_named_types()` - collect structs/enums for type definitions
- Any function using `TypeDetail`, `VariantDetail`, or `VariantPayload`

### 2. Go Codegen (`rust/roam-codegen/src/targets/go.rs`)

Same pattern as Rust. ~1554 lines.

### 3. Swift Codegen (`rust/roam-codegen/src/targets/swift.rs`)

Same pattern. ~1169 lines. Also uses `FieldDetail` which is now gone - use `facet_core::Field` directly.

### 4. Clean up `roam-reflect`

After codegen is done, `roam-reflect` can likely be deleted or significantly simplified since it was converting `Shape` → `TypeDetail`.

### 5. Update `roam-macros`

Change from emitting `type_detail::<T>()` to `<T as Facet>::SHAPE`. Look at `generate_method_details()` in `rust/roam-macros/src/lib.rs`.

## Key Types Reference

### roam-schema helpers

```rust
// Classification
pub fn classify_shape(shape: &'static Shape) -> ShapeKind<'static>
pub fn classify_variant(variant: &facet_core::Variant) -> VariantKind<'_>

// Predicates
pub fn is_tx(shape: &Shape) -> bool
pub fn is_rx(shape: &Shape) -> bool
pub fn is_stream(shape: &Shape) -> bool
pub fn is_bytes(shape: &Shape) -> bool

// ShapeKind variants
ShapeKind::Scalar(ScalarType)
ShapeKind::List { element }
ShapeKind::Array { element, len }
ShapeKind::Option { inner }
ShapeKind::Map { key, value }
ShapeKind::Set { element }
ShapeKind::Struct(StructInfo)
ShapeKind::Enum(EnumInfo)
ShapeKind::Tuple { elements }
ShapeKind::Tx { inner }
ShapeKind::Rx { inner }
ShapeKind::Pointer { pointee }
ShapeKind::Opaque

// VariantKind
VariantKind::Unit
VariantKind::Newtype { inner }
VariantKind::Tuple { fields }
VariantKind::Struct { fields }
```

### facet_core types

```rust
// Access struct fields
struct_type.fields  // &[Field]
field.name          // &str
field.shape()       // &'static Shape

// Access enum variants  
enum_type.variants  // &[Variant]
variant.name        // &str
variant.data        // StructType (use .kind and .fields)
```

## Current Branch

`tx-rx-rename` - pushed to origin

## Commands

```bash
# Check if Rust codegen compiles
cargo build -p roam-codegen 2>&1 | head -50

# Run tests after fixing
cargo test -p roam-codegen

# Full workspace check
cargo build --workspace
```
