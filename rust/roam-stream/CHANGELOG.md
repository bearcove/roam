# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-stream-v0.6.0...roam-stream-v4.0.0) - 2026-02-13

### Added

- [**breaking**] wire protocol v3 with metadata flags ([#65](https://github.com/bearcove/roam/pull/65))
- *(session)* Add non-generic Caller methods to reduce monomorphization ([#61](https://github.com/bearcove/roam/pull/61))

### Fixed

- make forwarded close handling explicit-event based ([#107](https://github.com/bearcove/roam/pull/107))
- restore Hello enum V1/V2 variants for wire compatibility

### Other

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
