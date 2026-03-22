# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/roam-types-v7.3.0...roam-types-v8.0.0) - 2026-03-22

### Added

- TypeScript schema exchange — codegen-driven CBOR schemas for wire protocol
- wire schema-aware deserialization with translation plans
- add SchemaMessage variant to MessagePayload for CBOR schema batches
- add schema_exchange field to Hello and HelloYourself handshake messages

### Fixed

- fixme(garbage) helpers

### Other

- Purge live v7 naming from SHM surface
- Split schema model out of roam-types
- Track channel retry mode in request metadata
- Fix native Rust tests after wasm runtime unification
- Unify the main Rust runtime on wasm
- Wait for late channel bindings
- Fix TS retry and generic schema extraction
- Use canonical schemas for TS channel and response paths
- Align schema exchange and channel retry behavior
- Align byte-buffer schema compatibility across Rust and TS
- Align TS canonical payload and channel encoding
- Split schema extraction from send tracking
- Forward schemas for proxied postcard payloads
- Outgoing/Incoming => Value/PostcardBytes
- Fix schema tracking scope and method root extraction
- Fix inline channel binding on send
- Update schema tests for root type refs
- Migrate schema bindings to ambient request context
- Add args_type
- do NOT share cycle schema index with tracker, jfc
- Redesign operation store and schema send path
- Redesign OperationStore trait and driver operation handling
- emit Schema objects as TypeScript literals instead of CBOR blobs
- Add internally-tagged enum support to facet-cbor and unify schema wire format
- TypeSchemaId => SchemaHash
- Fix duplicate schema IDs in schema payloads
- Remove RpcPlan/sidecar channel binding from driver and macros
- Panic on Rx::recv: facet_postcard usage is a bug
- Bind channels during ser/deser via thread-local ChannelBinder
- Simplify channels greatly
- Inline channel IDs in payload and bind during deserialization
- Remove const N generic from Tx/Rx, hardcode initial credit to 16
- Add CBOR round-trip test for MethodSchemaBinding with TypeRef
- Send root TypeRef in MethodSchemaBinding instead of bare TypeSchemaId
- Return root TypeRef from extract_schemas, resolve Vars in plan builder
- Deduplicate schemas by DeclId, emit generic type parameters
- Add SchemaKind::Channel with direction, element type, and initial credit
- Add CycleSchemaIndex and TypeParamName newtypes, align spec
- Newtypes are cool
- Fix test failures: type_ref on wire, base type names, acceptor rename
- SchemaHasher context + generic TypeRef + no naked u64s
- add TypeRef to schema types, use in all type reference positions
- typestate for schema IDs: MixedId during extraction, TypeSchemaId after
- use finalize_content_hashes for constructed schemas
- implement blake3 content hashing for TypeSchemaId
- align schema types between spec and Rust implementation
- It's forbidden to return schemas from RPC methods, therefore, we don't need channels on the RequestResponse struct
- phase 3: opaque u32le payload framing — fix codegen, schema, codec, golden vectors
- Refine retry semantics
- Disable dlog macro, revert write_vectored on LinkTxPermit
- Fix unsound OwnedMetadata::store_string/store_bytes returning arbitrary lifetime
- Add RecvResult type alias to eliminate type_complexity allow on ConduitRx::recv
- Replace manual_async_fn patterns with async fn, use CallResult alias everywhere
- Add MaybeSendFuture trait, BoxFut/CallResult aliases, box TranslationErrorKind
- Fix clippy warnings: collapsible ifs, redundant closures, type aliases, approx constants
- WIP retry with schemas
- Per-message SchemaRecvTracker, WithTracker on Caller::call, per-connection send state
- Gift correct data structures
- Clean up SchemaSendTracker: remove dead mutexes, fast-path method check, use MethodId as map key
- Split SchemaTracker into SchemaSendTracker + SchemaRecvTracker, inline schemas in RequestCall/RequestResponse
- CBOR handshake replaces postcard Hello/HelloYourself
- Remove #[facet(trailing)] — all opaque fields are length-prefixed
- Schema-vs-schema plan building, type names in schemas
- Merge roam-schema + roam-schema-extract into roam-types
- CBOR handshake, per-connection schema tracking, flexible wire protocol
- WIP backwards compat
- Remove schema_exchange from wire protocol — always-on in v9
- Fix all clippy warnings and errors across workspace
- extract roam-schema-extract crate, fix sentinel passthrough detection
- migrate roam from facet-postcard to roam-postcard
- Add channel retry semantics matrix coverage ([#251](https://github.com/bearcove/roam/pull/251))
- Align SHM with v9 transport prologue ([#246](https://github.com/bearcove/roam/pull/246))
- Implement automatic retry after session resume ([#239](https://github.com/bearcove/roam/pull/239))
- Add manual session resumption on a new conduit ([#237](https://github.com/bearcove/roam/pull/237))
- Implement retry operation identity core ([#236](https://github.com/bearcove/roam/pull/236))
- Define static retry policies ([#235](https://github.com/bearcove/roam/pull/235))
- Add Rust client middleware and logging ([#230](https://github.com/bearcove/roam/pull/230))
- Add Rust server middleware and logging ([#229](https://github.com/bearcove/roam/pull/229))
- Add opt-in request context for Rust service methods ([#228](https://github.com/bearcove/roam/pull/228))

## [7.0.0-alpha.3](https://github.com/bearcove/roam/compare/roam-types-v7.0.0-alpha.2...roam-types-v7.0.0-alpha.3) - 2026-03-03

### Other

- Add MaybeSend bound on erased caller
