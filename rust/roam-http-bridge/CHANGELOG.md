# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-http-bridge-v0.6.0...roam-http-bridge-v4.0.0) - 2026-02-13

### Added

- [**breaking**] wire protocol v3 with metadata flags ([#65](https://github.com/bearcove/roam/pull/65))

### Fixed

- make forwarded close handling explicit-event based ([#107](https://github.com/bearcove/roam/pull/107))

### Other

- Fix cargo-shear: clean up unused/misplaced deps in roam-miri-test and roam-http-bridge
- Fix handle_close: scope MutexGuard before await
- Fix MutexGuard held across await in handle_data; fix roam-shm try_read → try_lock
- Migrate roam-http-bridge from tokio::sync::Mutex to std::sync::Mutex
- Improve roam task instrumentation via tracked spawns
- Instrument tokio sync primitives with peeps-sync ([#135](https://github.com/bearcove/roam/pull/135))
- Fix concurrency safety across bipbuf, WS handler, tracing cache, and SHM recv ([#134](https://github.com/bearcove/roam/pull/134))
