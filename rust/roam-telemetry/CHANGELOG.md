# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-telemetry-v3.0.0...roam-telemetry-v4.0.0) - 2026-02-21

### Added

- [**breaking**] wire protocol v3 with metadata flags ([#65](https://github.com/bearcove/roam/pull/65))
- log user error details in telemetry middleware
- improve RPC error observability
- *(telemetry)* make TLS backend configurable
- *(session)* Add non-generic Caller methods to reduce monomorphization ([#61](https://github.com/bearcove/roam/pull/61))

### Fixed

- *(telemetry)* use option_env for CARGO_PKG_VERSION

### Other

- Pass &'static MethodDescriptor through the entire call chain ([#148](https://github.com/bearcove/roam/pull/148))
- Introduce ServiceDescriptor with precomputed RpcPlans ([#147](https://github.com/bearcove/roam/pull/147))
- Unify WASM/non-WASM duplicated code with macros (fixes #138)
- migrate roam runtime/facade plumbing to moire APIs
- migrate roam integration to moire dependency names
- Generate service facades and thread SourceId through RPC call paths
- huh
- migrate deprecated peeps fn calls to macros
- Migrate roam-telemetry export interval to peeps::interval
- Capture method_name _properly_
- More peeps instrumentation
- Improve roam task instrumentation via tracked spawns
- Load testing improvements and memory safety fixes ([#136](https://github.com/bearcove/roam/pull/136))
- Instrument tokio sync primitives with peeps-sync ([#135](https://github.com/bearcove/roam/pull/135))
- Replace recursive walkers with precomputed RpcPlan, rename stream to channel ([#85](https://github.com/bearcove/roam/pull/85))
- Fix/otlp skip none fields ([#67](https://github.com/bearcove/roam/pull/67))
- Add per-argument span attributes via Context.arg_names ([#56](https://github.com/bearcove/roam/pull/56))
- Add roam-telemetry crate and pre/post middleware support ([#55](https://github.com/bearcove/roam/pull/55))
