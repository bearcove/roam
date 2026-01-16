# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0](https://github.com/bearcove/roam/releases/tag/roam-stream-v0.6.0) - 2026-01-16

### Added

- *(wire)* add channels field to Request for transparent proxying ([#37](https://github.com/bearcove/roam/pull/37))
- *(session)* improve RoutedDispatcher API with automatic method routing ([#33](https://github.com/bearcove/roam/pull/33))
- make generated clients generic over Caller trait for reconnection support ([#8](https://github.com/bearcove/roam/pull/8))
- *(roam-stream)* add ReconnectingClient for automatic reconnection ([#7](https://github.com/bearcove/roam/pull/7))
- Add browser tests for streaming RPC methods
- Add complex type support for TypeScript codegen
- Add WebSocket transport support for roam
- Achieve 100% spec coverage for roam/rust implementation
- Add spec coverage for metadata and lifecycle rules
- Add spec coverage for streaming lifecycle and transport rules
- Add spec coverage for unary errors, request lifecycle, and cancel

### Fixed

- flatten nested Result types (closes #19) ([#21](https://github.com/bearcove/roam/pull/21))
- Correct streaming type inversion per spec r[streaming.caller-pov]

### Other

- Add version requirements for workspace publish
- Fix doorbell on Windows
- Some Windows fixes
- Remove idle timeout, use proper async waiting
- *(roam-stream)* [**breaking**] unify connection API with accept()/connect() ([#12](https://github.com/bearcove/roam/pull/12))
- rename unary.* rules to call.* throughout codebase ([#10](https://github.com/bearcove/roam/pull/10))
- rename Tx/Rx, migrate codegen to modular structure, add wire protocol ([#4](https://github.com/bearcove/roam/pull/4))
- Move RoutedDispatcher to roam-session and improve TypeScript codegen
- Take owned Vec<u8> in dispatch_streaming to avoid copy
- Rename roam-tcp to roam-stream and make transport generic
