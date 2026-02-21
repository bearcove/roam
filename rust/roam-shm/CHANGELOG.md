# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-shm-v0.6.0...roam-shm-v4.0.0) - 2026-02-21

### Added

- add directional roam connection instrumentation
- [**breaking**] wire protocol v3 with metadata flags ([#65](https://github.com/bearcove/roam/pull/65))
- *(shm)* Implement backpressure queue limits and Clean Abort strategy

### Fixed

- update payload size thresholds in queuing order test
- fix roam-shm peeps prelude import

### Other

- Pass &'static MethodDescriptor through the entire call chain ([#148](https://github.com/bearcove/roam/pull/148))
- Introduce ServiceDescriptor with precomputed RpcPlans ([#147](https://github.com/bearcove/roam/pull/147))
- Refactor Swift code to use rust over FFI instead of using C code ([#146](https://github.com/bearcove/roam/pull/146))
- Remove dead task context propagation (closes #141)
- Fix clippy warnings and remove unused dependencies
- Do not enable moire/diagnostics by default.
- ha
- migrate roam runtime/facade plumbing to moire APIs
- migrate roam integration to moire dependency names
- Use dispatcher method descriptors instead of runtime method-name registry
- huh
- use peeps instrument_future_on for response causality
- migrate deprecated peeps fn calls to macros
- typed peeps rpc handles for request/response diagnostics
- centralize request/response diagnostics and cheap context touch
- route diagnostics emission through shared spy
- Replace remaining response.created_at_ns alias in roam-shm
- Fix shear and strict clippy failures across session/stream/shm
- Use canonical peeps attrs in roam-shm diagnostics
- Try to fix request lifecycle
- Fix cfg gates
- transport-level accuracy for reqs/responses
- no early node deletion for RPC
- Migrate roam-shm timeouts to peeps::timeout
- Mark cancelled response nodes in roam-shm with close metadata
- Add cancellation metadata and roam-shm close handling, update dependencies
- key inflight requests by (conn_id, request_id)
- Capture method_name _properly_
- Remove old snapshot code
- More peeps instrumentation
- link server channel opens to request context
- update roam-shm request diagnostics for task metadata
- Convert tokio::spawn to peeps_tasks::spawn_tracked in roam-shm
- Fix MutexGuard held across await in handle_data; fix roam-shm try_read → try_lock
- Add clippy.toml banning async mutexes and RwLocks; migrate RwLock→Mutex in diagnostic, tracing, and shm crates
- Load testing improvements and memory safety fixes ([#136](https://github.com/bearcove/roam/pull/136))
- Instrument tokio sync primitives with peeps-sync ([#135](https://github.com/bearcove/roam/pull/135))
- Fix concurrency safety across bipbuf, WS handler, tracing cache, and SHM recv ([#134](https://github.com/bearcove/roam/pull/134))
- bak--
- More peep support ([#132](https://github.com/bearcove/roam/pull/132))
- Hello V6 protocol + diagnostic improvements ([#127](https://github.com/bearcove/roam/pull/127))
- shm bootstrap: atomically pass hub+doorbell fds across Rust/Swift ([#123](https://github.com/bearcove/roam/pull/123))
- Fix Swift SHM channeling request framing (issue #120) ([#122](https://github.com/bearcove/roam/pull/122))
- Add pull-based SHM diagnostics + stabilize Swift SHM interop ([#119](https://github.com/bearcove/roam/pull/119))
- Fix #111: Swift SHM guest runtime + VarSlotPool + doorbell + remap ([#115](https://github.com/bearcove/roam/pull/115))
- Fix SHM wrap invariant in spec + Swift shared-memory runtime ([#114](https://github.com/bearcove/roam/pull/114))
- Derisk SHM bootstrap with Rust↔Swift doorbell FD handoff ([#113](https://github.com/bearcove/roam/pull/113))
- SHM transport: decouple from roam-frame Frame type ([#108](https://github.com/bearcove/roam/pull/108))
- SHM transport v2: BipBuffer + shared VarSlotPool ([#104](https://github.com/bearcove/roam/pull/104))
- Rust session driver: make silent drops observable and fail loud on unknown conn_id ([#97](https://github.com/bearcove/roam/pull/97))
- Swift driver: make silent failure paths fail loud ([#96](https://github.com/bearcove/roam/pull/96))
- Remove COBS framing from protocol v4 ([#84](https://github.com/bearcove/roam/pull/84))
- Minimize monomorphization with Shape-based dispatch ([#54](https://github.com/bearcove/roam/pull/54))
- Divide roam-session into several files, use decl_id properly
