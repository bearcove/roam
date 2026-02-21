# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-stream-v0.6.0...roam-stream-v4.0.0) - 2026-02-21

### Added

- [**breaking**] wire protocol v3 with metadata flags ([#65](https://github.com/bearcove/roam/pull/65))
- *(session)* Add non-generic Caller methods to reduce monomorphization ([#61](https://github.com/bearcove/roam/pull/61))

### Fixed

- make forwarded close handling explicit-event based ([#107](https://github.com/bearcove/roam/pull/107))
- restore Hello enum V1/V2 variants for wire compatibility

### Other

- Pass &'static MethodDescriptor through the entire call chain ([#148](https://github.com/bearcove/roam/pull/148))
- Introduce ServiceDescriptor with precomputed RpcPlans ([#147](https://github.com/bearcove/roam/pull/147))
- Preserve specific ConnectFailed error in call_raw retry loop (closes #130)
- Unify WASM/non-WASM duplicated code with macros (fixes #138)
- migrate roam runtime/facade plumbing to moire APIs
- migrate roam integration to moire dependency names
- Generate service facades and thread SourceId through RPC call paths
- Use dispatcher method descriptors instead of runtime method-name registry
- Migrate roam session call paths to SourceRight
- huh
- migrate deprecated peeps fn calls to macros
- Fix shear and strict clippy failures across session/stream/shm
- Remove redundant peepable wrappers from first-class resources
- Migrate roam-stream sleep/timeout to peeps wrappers
- Use peeps::net::connect in roam-stream driver
- Remove old examples
- Capture method_name _properly_
- More peeps instrumentation
- instrument socket waits with peepable futures
- Migrate roam-stream to DiagnosticMutex
- Fix remaining clippy warnings in examples and tests
- Migrate roam-stream from tokio::sync::Mutex and RwLock to std::sync::Mutex
- Improve roam task instrumentation via tracked spawns
- Load testing improvements and memory safety fixes ([#136](https://github.com/bearcove/roam/pull/136))
- Fix silently ignored response channel binding in roam-stream Client ([#133](https://github.com/bearcove/roam/pull/133))
- Hello V6 protocol + diagnostic improvements ([#127](https://github.com/bearcove/roam/pull/127))
- Add diagnostics feature ([#116](https://github.com/bearcove/roam/pull/116))
- Cache Message TypePlan, remove AFL fuzz harness ([#86](https://github.com/bearcove/roam/pull/86))
- Replace recursive walkers with precomputed RpcPlan, rename stream to channel ([#85](https://github.com/bearcove/roam/pull/85))
- Remove COBS framing from protocol v4 ([#84](https://github.com/bearcove/roam/pull/84))
- Improve AFL harness focus
- Improve fuzzing seeds and coverage
- Optimize CobsFramed::send to avoid double allocation (fixes #59)
- shed ctor dependency, which isn't fuzzing friendly
- Minimize monomorphization with Shape-based dispatch ([#54](https://github.com/bearcove/roam/pull/54))
- Divide roam-session into several files, use decl_id properly
