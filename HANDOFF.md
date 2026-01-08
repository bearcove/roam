# Handoff: Replacing TypeDetail with facet::Shape

## Context

We're working on GitHub issue #2: removing the custom `TypeDetail` type system in favor of using `facet::Shape` directly.

**Why**: `TypeDetail` is a less precise copy of facet's `Shape`. Since facet already provides complete type introspection via `Shape`, maintaining a parallel type system is unnecessary overhead.

## Architecture Decision: Proc-Macro is Metadata-Only

See `CODEGEN.md` for full details. The key decision:

- **`roam-macros`** emits ONLY `{service}_service_detail() -> ServiceDetail`
- **`roam-codegen`** generates ALL client/server code (including Rust) from `ServiceDetail`
- The trait syntax is an input DSL only - it's not re-emitted

## Completed Work

### 1. Core Migration ✅

- **Updated facet dependency** - `cargo update -p facet` pulled in latest with `computed_variance()` API
- **Removed `roam-reflect`** - Directory deleted, no longer needed since we use `Shape` directly
  - Removed from `rust/roam/Cargo.toml`
  - Removed `pub use roam_reflect as reflect;` from `rust/roam/src/lib.rs`

### 2. roam-macros Updated ✅

- Changed `type_detail_expr()` to `shape_expr()` - now generates `<T as Facet>::SHAPE`
- Fixed field name: `type_info` → `ty` to match `ArgDetail`
- Fixed `Cow::as_str()` usage (unstable) → `as_ref()`

### 3. roam-session Tx/Rx ✅

- **Manual Facet impl** for `Tx<T>` and `Rx<T>` with:
  - `module_path("roam_session")` 
  - `type_params` properly exposing `T`
  - Proper variance handling
- Removed old attribute-based markers (`roam::tx`, `roam::rx`)

### 4. roam-schema Updated ✅

- `is_tx()` and `is_rx()` now use `module_path` + `type_identifier` comparison
- Added `fully_qualified_type_path()` helper function
- Added `ShapeKind::Result { ok, err }` variant for Result<T, E> types
- `classify_shape()` now handles `Def::Result` from facet

### 5. roam-frame Fixed ✅

- Fixed variance check: `(T::SHAPE.variance)(T::SHAPE)` → `T::SHAPE.computed_variance()`
- Fixed `shape.id` → `shape.type_identifier` in assert message

### 6. Rust Codegen Updated ✅

- **`CallResult<T, E>`** - Now properly generates with error type parameter
- **`RoamError<E>`** - Now properly generates with error type parameter  
- **`ServiceDispatcher` impl** - Added trait implementation for generated dispatchers
- **Handler traits** - Changed from `async fn` to `fn -> impl Future + Send` for proper Send bounds
- **Argument decoding** - Fixed to decode tuple of all args at once (not individually)
- **Result type handling** - `rust_type_base()` now properly handles `ShapeKind::Result`
- Added helper functions:
  - `extract_result_types()` - Detects `Result<T, E>` and extracts types
  - `format_caller_return_type()` - Generates `CallResult<T, E>` or `CallResult<T, Never>`
  - `format_handler_return_type()` - Generates `Result<T, RoamError<E>>`

### 7. Handler Implementations Updated ✅

Updated to use `RoamError<Never>` instead of `Box<dyn Error>`:
- `rust/subject-rust/src/main.rs`
- `rust/codegen-test-consumer/src/bin/ws_server.rs`
- `rust/codegen-test-consumer/src/lib.rs`
- `spec/spec-tests/src/bin/tcp_echo_server.rs`
- `spec/spec-tests/src/bin/ws_echo_server.rs`

### 8. Test Files Updated ✅

- `rust/roam-codegen/tests/method_ids.rs` - Uses Shape instead of TypeDetail
- `spec/spec-tests/tests/echo.rs` - Uses Shape instead of TypeDetail
- All codegen tests pass including Result type handling

## All Tests Pass ✅

```bash
cargo build --workspace  # Clean build
cargo test --workspace   # All tests pass
```

## Key Files Changed

| File | Change |
|------|--------|
| `rust/roam/Cargo.toml` | Removed `roam-reflect` dep |
| `rust/roam/src/lib.rs` | Removed `reflect` re-export |
| `rust/roam-macros/src/lib.rs` | Uses `<T as Facet>::SHAPE`, fixed `as_ref()` |
| `rust/roam-session/src/lib.rs` | Manual Facet impl for Tx/Rx with module_path |
| `rust/roam-schema/src/lib.rs` | `is_tx`/`is_rx` use module_path, added `ShapeKind::Result` |
| `rust/roam-frame/src/owned_message.rs` | Fixed variance API |
| `rust/roam-codegen/src/targets/rust.rs` | Handler Send bounds, tuple decoding, Result handling |
| `rust/roam-codegen/src/targets/*.rs` | Added `ShapeKind::Result` handling to all targets |
| `rust/roam-codegen/Cargo.toml` | Added `facet` as dev-dependency for tests |
| Various test files | Updated to use Shape instead of TypeDetail |

## What Could Be Done Next

### 1. **Strip more from proc-macro** (optional cleanup)

Per `CODEGEN.md`, the proc-macro should eventually emit ONLY `service_detail()`:
- Remove `generate_method_ids()` 
- Remove `generate_dispatch_arms()`
- Remove `generate_client_methods()`
- Remove `CalculatorClient<C>` generation
- Remove `calculator_dispatch_unary()` generation

This can be done in a follow-up PR.

### 2. **Streaming dispatch implementation**

The `generate_dispatch_streaming()` function currently returns a TODO placeholder.

## Related Issues/PRs

- https://github.com/facet-rs/facet/issues/1716 - Request for `fully_qualified_type_path()` method
- Future: facet `decl_id` for comparing generic type declarations

## Commands

```bash
# Build workspace
cargo build --workspace

# Run all tests
cargo test --workspace

# Check specific crate
cargo check -p roam-codegen

# Run codegen tests
cargo test -p roam-codegen --lib
```

## Current Branch

`tx-rx-rename` - pushed to origin