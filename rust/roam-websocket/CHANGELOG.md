# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-websocket-v0.6.0...roam-websocket-v4.0.0) - 2026-02-21

### Added

- add WebAssembly build and browser tests ([#64](https://github.com/bearcove/roam/pull/64))

### Other

- Refactor Swift code to use rust over FFI instead of using C code ([#146](https://github.com/bearcove/roam/pull/146))
- More wasm fixes
- migrate roam runtime/facade plumbing to moire APIs
- migrate roam integration to moire dependency names
- migrate deprecated peeps fn calls to macros
- Migrate roam-websocket native recv_timeout to peeps::timeout
- Migrate roam-session runtime sleep/timeout to peeps wrappers
- More peeps instrumentation
- Improve roam task instrumentation via tracked spawns
- Hello V6 protocol + diagnostic improvements ([#127](https://github.com/bearcove/roam/pull/127))
- Add wasm MessageConnector + ws_connect helpers for roam-websocket ([#118](https://github.com/bearcove/roam/pull/118))
- Remove COBS framing from protocol v4 ([#84](https://github.com/bearcove/roam/pull/84))
