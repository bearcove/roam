# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-session-v0.6.0...roam-session-v4.0.0) - 2026-01-28

### Added

- [**breaking**] wire protocol v3 with metadata flags ([#65](https://github.com/bearcove/roam/pull/65))
- add WebAssembly build and browser tests ([#64](https://github.com/bearcove/roam/pull/64))
- improve RPC error observability
- *(session)* Add non-generic Caller methods to reduce monomorphization ([#61](https://github.com/bearcove/roam/pull/61))

### Fixed

- restore Hello enum V1/V2 variants for wire compatibility
- proper error handling in TypeScript codegen
- *(dispatch)* Reduce monomorphization in dispatch_call functions ([#62](https://github.com/bearcove/roam/pull/62)) ([#63](https://github.com/bearcove/roam/pull/63))

### Other

- Update for facet FormatDeserializer API change
- Upgrade to latest facet
- Add per-argument span attributes via Context.arg_names ([#56](https://github.com/bearcove/roam/pull/56))
- Add roam-telemetry crate and pre/post middleware support ([#55](https://github.com/bearcove/roam/pull/55))
- Minimize monomorphization with Shape-based dispatch ([#54](https://github.com/bearcove/roam/pull/54))
- Replace Never shim with std::convert::Infallible
- Divide roam-session into several files, use decl_id properly
- Split up roam-session a bit
