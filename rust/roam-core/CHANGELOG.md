# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/roam-core-v7.3.0...roam-core-v8.0.0) - 2026-03-22

### Added

- wire schema-aware deserialization with translation plans
- wire schema exchange into session and driver
- handle incoming SchemaMessage in Session::handle_message
- store schema_exchange negotiation result in Session
- add schema_exchange field to Hello and HelloYourself handshake messages

### Fixed

- add schema_exchange field to resume handshake paths

### Other

- Purge live v7 naming from SHM surface
- Rename wire golden fixture directory
- Fix browser channel-order flake
- Drop legacy TypeScript schema runtime paths
- Fix wasm websocket sends and browser resume tests
- Track channel retry mode in request metadata
- Fix native Rust tests after wasm runtime unification
- Unify the main Rust runtime on wasm
- Align schema exchange and channel retry behavior
- Align TS canonical payload and channel encoding
- Forward schemas for proxied postcard payloads
- Outgoing/Incoming => Value/PostcardBytes
- Fix inline channel binding on send
- Migrate schema bindings to ambient request context
- Add args_type
- Redesign operation store and schema send path
- Redesign OperationStore trait and driver operation handling
- Add regression test for per-connection schema recv tracker
- Move SchemaRecvTracker from session to per-connection
- Add dlog to session recv loop and close_all_connections
- Fix duplicate schema IDs in schema payloads
- Fix test compilation: update credit test, accept macro snapshots
- Remove RpcPlan/sidecar channel binding from driver and macros
- Simplify channels greatly
- Fix compilation errors from const N removal in tests
- Clippy makes me unhappy
- Remove const N generic from Tx/Rx, hardcode initial credit to 16
- Add map golden vectors and TypeScript cross-language map tests
- Add composite golden vectors and cross-language TypeScript conformance tests
- Return root TypeRef from extract_schemas, resolve Vars in plan builder
- Deduplicate schemas by DeclId, emit generic type parameters
- Add CycleSchemaIndex and TypeParamName newtypes, align spec
- Fix errors redo codegen
- It's forbidden to return schemas from RPC methods, therefore, we don't need channels on the RequestResponse struct
- Adapt tests to new naming
- phase 3: opaque u32le payload framing — fix codegen, schema, codec, golden vectors
- align API names with Rust — initiatorConduit/acceptorConduit, add initiatorOnLink/acceptorOnLink, remove acceptor alias
- Finish resumable response-path and auto-resume coverage
- Refine retry semantics
- Replace local BoxFuture with BoxFut, introduce RecoveredConduit struct
- Replace manual_async_fn patterns with async fn, use CallResult alias everywhere
- Add MaybeSendFuture trait, BoxFut/CallResult aliases, box TranslationErrorKind
- Fix clippy warnings: collapsible ifs, redundant closures, type aliases, approx constants
- WIP retry with schemas
- Fix response schema attachment broken by operation dedup path
- Per-message SchemaRecvTracker, WithTracker on Caller::call, per-connection send state
- Gift correct data structures
- Add schema resume integration tests using real services and handshakes
- Stable conduit integration tests and acceptor role fix
- Schema validation: type name checks, SchemaPath newtype, rich error types
- Clean up SchemaSendTracker: remove dead mutexes, fast-path method check, use MethodId as map key
- use Payload for frame item, re-enable CBOR handshake paths
- Wire handshake schemas into message deserialization via MessagePlan
- Split SchemaTracker into SchemaSendTracker + SchemaRecvTracker, inline schemas in RequestCall/RequestResponse
- CBOR handshake replaces postcard Hello/HelloYourself
- Merge roam-schema + roam-schema-extract into roam-types
- WIP backwards compat
- Remove schema_exchange from wire protocol — always-on in v9
- Fix all clippy warnings and errors across workspace
- merge SchemaRegistry into SchemaTracker, remove Option
- enable schema exchange unconditionally in v9
- more tracing in driver/session cancel path
- add tracing to cancel/failure paths for debugging timeouts
- extract roam-schema-extract crate, fix sentinel passthrough detection
- migrate roam from facet-postcard to roam-postcard
- Add tracey annotations for schema exchange spec coverage
- Add channel retry semantics matrix coverage ([#251](https://github.com/bearcove/roam/pull/251))
- Align SHM with v9 transport prologue ([#246](https://github.com/bearcove/roam/pull/246))
- Add resumable acceptor registry and browser reconnect coverage ([#244](https://github.com/bearcove/roam/pull/244))
- Normalize initiator connector APIs ([#242](https://github.com/bearcove/roam/pull/242))
- Implement v9 transport prologue ([#241](https://github.com/bearcove/roam/pull/241))
- Implement automatic retry after session resume ([#239](https://github.com/bearcove/roam/pull/239))
- Add manual session resumption on a new conduit ([#237](https://github.com/bearcove/roam/pull/237))
- Implement retry operation identity core ([#236](https://github.com/bearcove/roam/pull/236))
- Define static retry policies ([#235](https://github.com/bearcove/roam/pull/235))
- Rewrite TypeScript runtime around layered sessions ([#233](https://github.com/bearcove/roam/pull/233))

### Changed

- Remove the implicit `From<DriverCaller> for ()` conversion and add `NoopCaller` for liveness-only root handles.

## [7.0.0-alpha.3](https://github.com/bearcove/roam/compare/roam-core-v7.0.0-alpha.2...roam-core-v7.0.0-alpha.3) - 2026-03-03

### Other

- Add MaybeSend bound on erased caller
