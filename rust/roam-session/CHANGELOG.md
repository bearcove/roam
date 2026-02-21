# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-session-v0.6.0...roam-session-v4.0.0) - 2026-02-21

### Added

- add directional roam connection instrumentation
- [**breaking**] wire protocol v3 with metadata flags ([#65](https://github.com/bearcove/roam/pull/65))
- add WebAssembly build and browser tests ([#64](https://github.com/bearcove/roam/pull/64))
- improve RPC error observability
- *(session)* Add non-generic Caller methods to reduce monomorphization ([#61](https://github.com/bearcove/roam/pull/61))

### Fixed

- make forwarded close handling explicit-event based ([#107](https://github.com/bearcove/roam/pull/107))
- *(roam-session)* Update deserialize_into calls for new Facet API
- *(roam-session)* Update to new Partial::from_raw API
- restore Hello enum V1/V2 variants for wire compatibility
- proper error handling in TypeScript codegen
- *(dispatch)* Reduce monomorphization in dispatch_call functions ([#62](https://github.com/bearcove/roam/pull/62)) ([#63](https://github.com/bearcove/roam/pull/63))

### Other

- Pass &'static MethodDescriptor through the entire call chain ([#148](https://github.com/bearcove/roam/pull/148))
- Introduce ServiceDescriptor with precomputed RpcPlans ([#147](https://github.com/bearcove/roam/pull/147))
- Remove FACET_BUG_LOCK from dispatch_call_infallible (closes #139)
- Unify WASM/non-WASM duplicated code with macros (fixes #138)
- use moire::sync::Semaphore on all targets including wasm
- Remove dead task context propagation (closes #141)
- Fix clippy warnings and remove unused dependencies
- Remove useless clippy annotation
- More weirdness
- hello_params => simplify return type
- Fix disallowed
- Add roam-memory transport and adapt timeout call sites
- migrate roam runtime/facade plumbing to moire APIs
- migrate roam integration to moire dependency names
- Generate service facades and thread SourceId through RPC call paths
- Pass typed peeps request/response bodies through roam session
- Use dispatcher method descriptors instead of runtime method-name registry
- Migrate roam session call paths to SourceRight
- *sigh* cleanups
- dedupe response diagnostics and enforce strict metadata
- huh
- migrate deprecated peeps fn calls to macros
- Add handshake-correlated connection scopes for peeps diagnostics
- Peep migration
- typed peeps rpc handles for request/response diagnostics
- centralize request/response diagnostics and cheap context touch
- stabilize connection diagnostics context id + remove link from token
- route diagnostics emission through shared spy
- add shared request/response diagnostics spy
- Lots more instrumentation
- Fix cfg-gated PendingResponse tests and Swift metadata callsites
- Apply pending tokio runtime session updates
- Fix shear and strict clippy failures across session/stream/shm
- checkpoint roam-session connection instrumentation
- model queued call edge to driver queue
- record absolute request/response timestamps for peeps
- Plumb Swift call metadata and drop unused peeps caller metadata
- Remove redundant peepable wrappers from first-class resources
- Try to fix request lifecycle
- Fix cfg gates
- transport-level accuracy for reqs/responses
- no early node deletion for RPC
- Migrate roam-session runtime sleep/timeout to peeps wrappers
- Add cancellation metadata and roam-shm close handling, update dependencies
- Mark cancelled response nodes with close metadata instead of removing
- key inflight requests by (conn_id, request_id)
- always link nested request spans and simplify stack ensure
- Migrate with_stack to ensure in roam-session
- unify request path for drained and non-drained calls
- Remove old examples
- various peeps fixes
- Fix borrows
- include request.args in peeps node attrs_json
- scope response recv under peeps stack; fix remaining test call signatures
- Capture method_name _properly_
- link outgoing request to caller; scope incoming handler under response
- scope request node on peeps stack during call_raw
- More peeps instrumentation
- link server channel opens to request context
- Propagate caller identity metadata on outgoing RPC calls
- Improve .peepable() labels for clarity and uniqueness
- propagate rpc chain/span metadata across calls
- record inbound rpc task context from metadata
- More propagation
- Context propagation for calls
- Fix cargo-shear: clean up unused/misplaced deps in roam-miri-test and roam-http-bridge
- Name all runtime::spawn calls in roam-session
- Migrate roam-session to DiagnosticMutex via runtime abstraction
- Fix diagnostic_snapshot.rs: try_read → try_lock for Mutex migration
- Migrate roam-session from tokio::sync::Mutex to std::sync::Mutex
- Add clippy.toml banning async mutexes and RwLocks; migrate RwLock→Mutex in diagnostic, tracing, and shm crates
- Improve roam task instrumentation via tracked spawns
- Load testing improvements and memory safety fixes ([#136](https://github.com/bearcove/roam/pull/136))
- Instrument tokio sync primitives with peeps-sync ([#135](https://github.com/bearcove/roam/pull/135))
- More peep support ([#132](https://github.com/bearcove/roam/pull/132))
- Hello V6 protocol + diagnostic improvements ([#127](https://github.com/bearcove/roam/pull/127))
- Add wasm MessageConnector + ws_connect helpers for roam-websocket ([#118](https://github.com/bearcove/roam/pull/118))
- Add diagnostics feature ([#116](https://github.com/bearcove/roam/pull/116))
- Add V5 request concurrency flow control ([#101](https://github.com/bearcove/roam/pull/101))
- Rust session driver: make silent drops observable and fail loud on unknown conn_id ([#97](https://github.com/bearcove/roam/pull/97))
- Fix payload size handling in Rust and Swift
- Replace recursive walkers with precomputed RpcPlan, rename stream to channel ([#85](https://github.com/bearcove/roam/pull/85))
- Remove COBS framing from protocol v4 ([#84](https://github.com/bearcove/roam/pull/84))
- make channel ID collection schema-driven ([#80](https://github.com/bearcove/roam/pull/80))
- Update to facet with MetaSource API change
- Update for facet FormatDeserializer API change
- Upgrade to latest facet
- Add per-argument span attributes via Context.arg_names ([#56](https://github.com/bearcove/roam/pull/56))
- Add roam-telemetry crate and pre/post middleware support ([#55](https://github.com/bearcove/roam/pull/55))
- Minimize monomorphization with Shape-based dispatch ([#54](https://github.com/bearcove/roam/pull/54))
- Replace Never shim with std::convert::Infallible
- Divide roam-session into several files, use decl_id properly
- Split up roam-session a bit
