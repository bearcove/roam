# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/roam-macros-core-v7.3.0...roam-macros-core-v8.0.0) - 2026-03-22

### Added

- wire schema-aware deserialization with translation plans

### Other

- Track channel retry mode in request metadata
- Fix borrowed macro dispatch and resume test
- Unify the main Rust runtime on wasm
- Align schema exchange and channel retry behavior
- Unignore TypeScript spec matrix lanes
- Forward schemas for proxied postcard payloads
- Fix schema tracking scope and method root extraction
- Fix test compilation: update credit test, accept macro snapshots
- Remove RpcPlan/sidecar channel binding from driver and macros
- Fix clippy warnings: collapsible ifs, redundant closures, type aliases, approx constants
- Per-message SchemaRecvTracker, WithTracker on Caller::call, per-connection send state
- Gift correct data structures
- Split SchemaTracker into SchemaSendTracker + SchemaRecvTracker, inline schemas in RequestCall/RequestResponse
- Merge roam-schema + roam-schema-extract into roam-types
- WIP backwards compat
- Add channel retry semantics matrix coverage ([#251](https://github.com/bearcove/roam/pull/251))
- Implement automatic retry after session resume ([#239](https://github.com/bearcove/roam/pull/239))
- Implement retry operation identity core ([#236](https://github.com/bearcove/roam/pull/236))
- Define static retry policies ([#235](https://github.com/bearcove/roam/pull/235))
- Add Rust client middleware and logging ([#230](https://github.com/bearcove/roam/pull/230))
- Add Rust server middleware and logging ([#229](https://github.com/bearcove/roam/pull/229))
- Add opt-in request context for Rust service methods ([#228](https://github.com/bearcove/roam/pull/228))
