# Swift Implementation Parity

## Goal

Bring the Swift implementation up to parity with Rust and TypeScript in terms of:
- Schema-driven serialization/deserialization
- Channel binding
- Connection logic
- Code generation from `roam-codegen`
- Spec compliance (tracey rule coverage)

## Current State (Updated after Phase 001)

The Swift implementation is **more complete than originally estimated**:

| Component | Rust | TypeScript | Swift | Notes |
|-----------|------|------------|-------|-------|
| Postcard primitives | ✅ | ✅ | ✅ | Working |
| Wire protocol | ✅ | ✅ | ✅ | All variants |
| COBS framing | ✅ | ✅ | ✅ | Working |
| SwiftNIO transport | N/A | N/A | ✅ | Working |
| Channel types (Tx/Rx) | ✅ | ✅ | ✅ | Actor-based |
| Channel registry | ✅ | ✅ | ✅ | Working |
| Driver/Connection | ✅ | ✅ | ✅ | Working |
| **Code generation** | N/A | ✅ | ✅ | **Functional!** |
| **Schema types** | ✅ | ✅ | ✅ | Generated |
| **Schema-driven encode** | ✅ | ✅ | ✅ | Generated |
| **Schema-driven decode** | ✅ | ✅ | ✅ | Generated |
| **Channel binding** | ✅ | ✅ | ✅ | Generated |
| **Client stubs** | ✅ | ✅ | ✅ | Generated |
| **Server dispatcher** | ✅ | ✅ | ✅ | Generated |
| **Spec rule coverage** | 80% | 47% | 0% | No annotations yet |
| **Golden vector tests** | ✅ | ✅ | ✅ | All pass |

### Spec Test Results

| Category | Pass | Fail | Notes |
|----------|------|------|-------|
| Protocol tests | 7/7 | 0 | All pass |
| Testbed (unary) | 4/4 | 0 | All pass |
| Client mode | 3/3 | 0 | All pass |
| **Streaming** | 0/3 | 3 | **Tx/Driver ordering bug** |
| **Total** | **16/17** | **1** | 94% pass rate |

## Key Finding: Streaming Bug

The only remaining issue is in streaming RPC. The problem:

1. `dispatchgenerate` creates `Tx` and calls `handler.generate(count, output)`
2. Handler calls `output.send(i)` which yields to `eventContinuation`
3. But dispatcher immediately calls `output.close()` and sends Response
4. Response reaches the wire before Data messages are processed

**Root cause**: `Tx.send()` is fire-and-forget; it yields to the async event stream but doesn't wait for the message to be sent. The dispatcher sends Response before the event loop processes pending Data messages.

## Revised Phases

Given the findings, the phase plan is significantly simplified:

| Phase | File | Status | Description |
|-------|------|--------|-------------|
| 001 | [001-DONE-assess-swift-baseline.md](./001-DONE-assess-swift-baseline.md) | **DONE** | Audit, 16/17 tests pass |
| 002 | 002-TODO-fix-streaming.md | TODO | Fix Tx/Driver ordering |
| 003 | 003-TODO-spec-annotations.md | TODO | Add tracey rule annotations |
| 004 | 004-TODO-spec-tests.md | TODO | Verify 100% spec tests pass |

**Phases 002-012 from original plan are obsolete** — codegen already works!

## Architecture

### How Swift Works (code generation)

```
roam-codegen (Rust)
        │
        ▼
Generated Swift (Testbed.swift):
  - Type definitions (structs, enums)
  - Schema constants
  - Encode/decode functions
  - Client stubs with channel binding
  - Server dispatcher with preregistration
        │
        ▼
Runtime (roam-runtime):
  - Postcard primitives
  - Wire protocol
  - Channel types (Tx/Rx)
  - Driver event loop
  - SwiftNIO transport
```

## Files

### Swift Runtime
- `swift/roam-runtime/Sources/RoamRuntime/` — Core runtime (working)
- `swift/roam-runtime/Tests/RoamRuntimeTests/` — 39 tests (all pass)

### Swift Subject  
- `swift/subject/Sources/subject-swift/Subject.swift` — Handler implementation
- `swift/subject/Sources/subject-swift/Testbed.swift` — Generated code

### Rust Codegen
- `rust/roam-codegen/src/targets/swift/` — Swift code generation (working)
  - `mod.rs` — Entry point
  - `types.rs` — Type generation
  - `schema.rs` — Schema constants
  - `encode.rs` — Encode expressions
  - `decode.rs` — Decode expressions
  - `client.rs` — Client stubs
  - `server.rs` — Server dispatcher

## Commands

```bash
# Run Swift runtime tests
cd swift/roam-runtime && swift test

# Build Swift subject
cd swift/subject && swift build -c release

# Run spec tests with Swift subject
SUBJECT_CMD="./swift/subject/.build/release/subject-swift" cargo test -p spec-tests

# Enable wire spy for debugging
ROAM_WIRE_SPY=1 SUBJECT_CMD="./swift/subject/.build/release/subject-swift" cargo test -p spec-tests

# Generate Swift code
cargo xtask codegen --swift
```

## Success Criteria

1. ✅ Swift golden vector tests pass
2. ✅ Swift subject builds and runs
3. ✅ 16/17 spec tests pass
4. ⏳ All streaming tests pass (Phase 002)
5. ⏳ Tracey annotations added (Phase 003)
6. ⏳ 100% spec test pass rate (Phase 004)

## Estimated Remaining Effort

| Phase | Complexity | Est. Time |
|-------|------------|-----------|
| 002 Fix Streaming | Medium | 2-4 hours |
| 003 Spec Annotations | Low | 2-3 hours |
| 004 Final Verification | Low | 1-2 hours |
| **Total** | | **5-9 hours** |

This is now a **single-session effort** instead of the originally estimated 44-64 hours.
