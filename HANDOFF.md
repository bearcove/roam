# Handoff: Replacing TypeDetail with facet::Shape

## Context

We're working on GitHub issue #2: removing the custom `TypeDetail` type system in favor of using `facet::Shape` directly.

**Why**: `TypeDetail` is a less precise copy of facet's `Shape`. Since facet already provides complete type introspection via `Shape`, maintaining a parallel type system is unnecessary overhead.

## What's Been Done

### 1. Core Infrastructure ✅

- **`roam-schema`**: Added `ShapeKind`, `StructInfo`, `EnumInfo`, `VariantKind` types and `classify_shape()` / `classify_variant()` functions. These provide the abstraction layer for codegen to work with Shape.
- **`roam-hash`**: Fully migrated to use `Shape`, `ScalarType`, and the facet-core API.
- **`roam-codegen/render.rs`**: Switched from custom `kebab()` to `heck::ToKebabCase`.

### 2. Codegen Targets ✅ COMPLETE

| Target     | Status | Notes |
|------------|--------|-------|
| TypeScript | ✅ Done | Fully migrated |
| Java       | ✅ Done | Fully migrated |
| Python     | ✅ Done | Fully migrated |
| **Rust**   | ✅ Done | Fully migrated |
| Go         | ✅ Done | Fully migrated |
| Swift      | ✅ Done | Fully migrated |

All codegen targets now use `&'static Shape` instead of `TypeDetail`, `classify_shape()` instead of `match ty { TypeDetail::X => ... }`, and `classify_variant()` instead of `VariantPayload`.

## What Needs To Be Done

### 1. **PRIORITY: Remove roam-reflect** (`rust/roam-reflect/`)

The `roam-reflect` crate converts `Shape` → `TypeDetail`. Since we no longer use `TypeDetail`, this crate is now unnecessary.

**Steps:**
1. Remove `roam-reflect` from `rust/roam/Cargo.toml` dependencies
2. Remove `pub use roam_reflect as reflect;` from `rust/roam/src/lib.rs`
3. Delete the `rust/roam-reflect/` directory
4. Remove from workspace `Cargo.toml`

### 2. **Update roam-macros** (`rust/roam-macros/src/lib.rs`)

The macro currently generates code that calls `roam::reflect::type_detail::<T>()` to get `TypeDetail`. This needs to change to use `<T as Facet>::SHAPE` directly.

**Current code in `generate_method_details()`:**
```rust
#roam::reflect::type_detail::<#ty_tokens>().unwrap_or_else(|e| {
    panic!("Failed to get type_detail for {}: {e}", stringify!(#ty_tokens))
})
```

**Should become:**
```rust
<#ty_tokens as ::facet::Facet>::SHAPE
```

**Key changes:**
- `arg.type_info` → `arg.ty` (ArgDetail field renamed)
- `method.return_type` is already `&'static Shape` in the new schema
- Remove the `unwrap_or_else` since `SHAPE` is a const, not a `Result`

### 3. **Update roam-schema types** (if needed)

Verify that `MethodDetail` and `ArgDetail` in `roam-schema` use `&'static Shape`:
- `ArgDetail.ty: &'static Shape` ✅ (already done)
- `MethodDetail.return_type: &'static Shape` ✅ (already done)

### 4. **Run full workspace build**

After completing the above:
```bash
cargo build --workspace
cargo test --workspace
```

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
# Check if codegen compiles
cargo check -p roam-codegen --lib

# Run codegen tests (currently blocked by roam-reflect)
cargo test -p roam-codegen --lib

# Full workspace check (after roam-reflect removal)
cargo build --workspace
cargo test --workspace
```

## Migration Pattern Used

For each codegen file, the pattern was:

1. **Update imports:**
   ```rust
   use facet_core::{ScalarType, Shape};
   use roam_schema::{
       EnumInfo, MethodDetail, ServiceDetail, ShapeKind, StructInfo, VariantKind,
       classify_shape, classify_variant, is_bytes, is_rx, is_tx,
   };
   ```

2. **Change function signatures:** `&TypeDetail` → `&'static Shape`

3. **Replace type matching:**
   - `match ty { TypeDetail::X => ... }` → `match classify_shape(shape) { ShapeKind::X => ... }`
   - `match &v.payload { VariantPayload::X => ... }` → `match classify_variant(v) { VariantKind::X => ... }`

4. **Update field access:**
   - `arg.type_info` → `arg.ty`
   - `field.type_info` → `field.shape()`

5. **Update tests:** Use `<T as Facet>::SHAPE` to get Shape for test types