# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/roam-v7.3.0...roam-v8.0.0) - 2026-03-14

### Other

- Align SHM with v9 transport prologue ([#246](https://github.com/bearcove/roam/pull/246))
- Add resumable acceptor registry and browser reconnect coverage ([#244](https://github.com/bearcove/roam/pull/244))
- Normalize initiator connector APIs ([#242](https://github.com/bearcove/roam/pull/242))
- Implement retry operation identity core ([#236](https://github.com/bearcove/roam/pull/236))
- Define static retry policies ([#235](https://github.com/bearcove/roam/pull/235))
- Add Rust client middleware and logging ([#230](https://github.com/bearcove/roam/pull/230))
- Add Rust server middleware and logging ([#229](https://github.com/bearcove/roam/pull/229))
- Add opt-in request context for Rust service methods ([#228](https://github.com/bearcove/roam/pull/228))

### Changed

- Remove the implicit `From<DriverCaller> for ()` conversion. Use `NoopCaller` with `establish::<NoopCaller>(...)` when you want root liveness without a root client API.
