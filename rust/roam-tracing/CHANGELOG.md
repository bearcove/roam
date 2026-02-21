# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-tracing-v0.6.0...roam-tracing-v4.0.0) - 2026-02-21

### Other

- migrate roam runtime/facade plumbing to moire APIs
- migrate roam integration to moire dependency names
- migrate deprecated peeps fn calls to macros
- Migrate roam-tracing to peeps sleep and Notify
- More peeps instrumentation
- Migrate roam-tracing to DiagnosticMutex
- Add clippy.toml banning async mutexes and RwLocks; migrate RwLock→Mutex in diagnostic, tracing, and shm crates
- Improve roam task instrumentation via tracked spawns
- Instrument tokio sync primitives with peeps-sync ([#135](https://github.com/bearcove/roam/pull/135))
- Fix concurrency safety across bipbuf, WS handler, tracing cache, and SHM recv ([#134](https://github.com/bearcove/roam/pull/134))
- Minimize monomorphization with Shape-based dispatch ([#54](https://github.com/bearcove/roam/pull/54))
