# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/shm-primitives-v0.6.0...shm-primitives-v4.0.0) - 2026-02-21

### Other

- Fix concurrency safety across bipbuf, WS handler, tracing cache, and SHM recv ([#134](https://github.com/bearcove/roam/pull/134))
- shm bootstrap: atomically pass hub+doorbell fds across Rust/Swift ([#123](https://github.com/bearcove/roam/pull/123))
- Fix SHM wrap invariant in spec + Swift shared-memory runtime ([#114](https://github.com/bearcove/roam/pull/114))
- SHM transport v2: BipBuffer + shared VarSlotPool ([#104](https://github.com/bearcove/roam/pull/104))
