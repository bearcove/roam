# Handoff: Replacing TypeDetail with facet::Shape

## Context

We're working on GitHub issue #2: removing the custom `TypeDetail` type system in favor of using `facet::Shape` directly.

**Why**: `TypeDetail` is a less precise copy of facet's `Shape`. Since facet already provides complete type introspection via `Shape`, maintaining a parallel type system is unnecessary overhead.

## What's Been Done

### 1. Created `roam-macros-parse` crate
- **Location**: `rust/roam-macros-parse/`
- Pure unsynn grammar for parsing service trait definitions
- Snapshot tests using datatest-stable + insta in `tests/fixtures/`
- Committed and pushed

### 2. Updated `roam-macros`
- Now depends on `roam-macros-parse`
- Added comprehensive documentation explaining:
  - The two-phase design (proc macro emits metadata → build script does codegen)
  - Why proc macros can't see into the type system (only tokens)
  - Why facet::Shape at runtime solves this

### 3. Started updating `roam-schema`
- **File**: `rust/roam-schema/src/lib.rs`
- Removed `TypeDetail`, `FieldDetail`, `VariantDetail`, `VariantPayload`
- Changed `MethodDetail.return_type` and `ArgDetail.ty` to `&'static Shape`
- Added helper functions: `is_tx()`, `is_rx()`, `is_stream()`, `contains_stream()`
- Uses `Cow<'static, str>` for string fields
- **This compiles successfully**

### 4. Started updating `roam-hash` (INCOMPLETE)
- **File**: `rust/roam-hash/src/lib.rs`
- Attempted to rewrite `encode_type()` → `encode_shape()`
- **FAILED** because I made incorrect assumptions about facet-core's API

## What Needs To Be Done

### 1. Fix `roam-hash` by understanding the actual facet-core API

The key types to understand are in facet-core:
- `Shape` - the main type description struct
- `Shape.ty: Type` - the type category
- `Shape.def: Def` - the type definition with variant-specific info
- `Shape.type_params` - generic parameters

**You need to look at**:
- How `Type` enum is structured (NOT `Type::Primitive(PrimitiveType::U8)` - I was wrong)
- How `Def` enum is structured (NOT `Def::Struct(...)` - I was wrong)
- How to detect primitives, structs, enums, containers
- How field/variant iteration works

The facet source is in the workspace at `/Users/amos/bearcove/facet/` - look at:
- `facet-core/src/types/ty/` for `Type`
- `facet-core/src/types/def/` for `Def`
- `facet-core/src/types/shape.rs` for `Shape`

### 2. Update remaining crates

After `roam-hash` compiles:

- **`roam-reflect`**: Can probably be mostly deleted - it was converting Shape→TypeDetail. Maybe keep a thin wrapper if needed.

- **`roam-codegen`**: Update to use Shape instead of TypeDetail for code generation. The targets are in `rust/roam-codegen/src/targets/`.

- **`roam-macros`**: Change from emitting `type_detail::<T>()` to `<T as Facet>::SHAPE`. Look at `generate_method_details()` in `rust/roam-macros/src/lib.rs`.

### 3. Fix dependent crates

These will break during the transition:
- `codegen-test-proto`
- `codegen-test-consumer`
- `spec-proto`
- `spec-tests`
- `subject-rust`

## Key Design Decisions

1. **`&'static Shape` not `Shape`**: Shapes are static, we just reference them
2. **`Cow<'static, str>` for strings**: Allows both static and owned strings
3. **Roam-specific types via attributes**: `Tx` and `Rx` are detected via `#[facet(roam::tx)]` and `#[facet(roam::rx)]` attributes on the type
4. **Signature hashing must be deterministic**: The encoding of shapes must produce identical bytes for identical types

## Current Branch

`tx-rx-rename` - already pushed to origin

## Files Modified (uncommitted)

- `rust/roam-schema/src/lib.rs` - Updated to use Shape (compiles)
- `rust/roam-schema/Cargo.toml` - Added facet-core dependency
- `rust/roam-hash/src/lib.rs` - Broken, needs proper facet-core API usage
- `rust/roam-hash/Cargo.toml` - Added facet-core and heck dependencies

## Command to Check Status

```bash
cd /Users/amos/bearcove/roam
cargo build -p roam-schema  # Should work
cargo build -p roam-hash    # Currently broken - this is where to start
```
