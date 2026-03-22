# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/roam-v7.3.0...roam-v8.0.0) - 2026-03-22

### Added

- wire schema-aware deserialization with translation plans

### Fixed

- fixme(garbage) helpers
- use initiator_conduit instead of initiator in service_macro_shared tests

### Other

- Split schema model out of roam-types
- Track channel retry mode in request metadata
- Fix borrowed macro dispatch and resume test
- Unify the main Rust runtime on wasm
- Align schema exchange and channel retry behavior
- Outgoing/Incoming => Value/PostcardBytes
- Redesign OperationStore trait and driver operation handling
- Add internally-tagged enum support to facet-cbor and unify schema wire format
- Fix test compilation: update credit test, accept macro snapshots
- Remove RpcPlan/sidecar channel binding from driver and macros
- Send root TypeRef in MethodSchemaBinding instead of bare TypeSchemaId
- Deduplicate schemas by DeclId, emit generic type parameters
- Adapt tests to new naming
- Replace manual_async_fn patterns with async fn, use CallResult alias everywhere
- WIP retry with schemas
- Per-message SchemaRecvTracker, WithTracker on Caller::call, per-connection send state
- Gift correct data structures
- Add schema resume integration tests using real services and handshakes
- Split SchemaTracker into SchemaSendTracker + SchemaRecvTracker, inline schemas in RequestCall/RequestResponse
- CBOR handshake replaces postcard Hello/HelloYourself
- Schema-vs-schema plan building, type names in schemas
- Merge roam-schema + roam-schema-extract into roam-types
- WIP backwards compat
- Fix all clippy warnings and errors across workspace
- Add missing SHM tests
- name-only method IDs + schema compat tests (all 4 failing)
- migrate roam from facet-postcard to roam-postcard
- Add SHM borrowed-return teardown survival tests for inline/slot-ref/mmap-ref
- Align SHM with v9 transport prologue ([#246](https://github.com/bearcove/roam/pull/246))
- Add resumable acceptor registry and browser reconnect coverage ([#244](https://github.com/bearcove/roam/pull/244))
- Normalize initiator connector APIs ([#242](https://github.com/bearcove/roam/pull/242))
- Implement retry operation identity core ([#236](https://github.com/bearcove/roam/pull/236))
- Define static retry policies ([#235](https://github.com/bearcove/roam/pull/235))
- Add Rust client middleware and logging ([#230](https://github.com/bearcove/roam/pull/230))
- Add Rust server middleware and logging ([#229](https://github.com/bearcove/roam/pull/229))
- Add opt-in request context for Rust service methods ([#228](https://github.com/bearcove/roam/pull/228))

### Changed

- Remove the implicit `From<DriverCaller> for ()` conversion. Use `NoopCaller` with `establish::<NoopCaller>(...)` when you want root liveness without a root client API.
