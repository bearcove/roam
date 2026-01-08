# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/bearcove/roam/releases/tag/roam-stream-v0.5.0) - 2026-01-08

### Added

- Add browser tests for streaming RPC methods
- Add complex type support for TypeScript codegen
- Add WebSocket transport support for roam
- Achieve 100% spec coverage for roam/rust implementation
- Add spec coverage for metadata and lifecycle rules
- Add spec coverage for streaming lifecycle and transport rules
- Add spec coverage for unary errors, request lifecycle, and cancel

### Fixed

- Correct streaming type inversion per spec r[streaming.caller-pov]

### Other

- Move RoutedDispatcher to roam-session and improve TypeScript codegen
- Take owned Vec<u8> in dispatch_streaming to avoid copy
- Rename roam-tcp to roam-stream and make transport generic
