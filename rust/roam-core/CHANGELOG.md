# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/roam-core-v7.3.0...roam-core-v8.0.0) - 2026-03-14

### Other

- Add channel retry semantics matrix coverage ([#251](https://github.com/bearcove/roam/pull/251))
- Align SHM with v9 transport prologue ([#246](https://github.com/bearcove/roam/pull/246))
- Add resumable acceptor registry and browser reconnect coverage ([#244](https://github.com/bearcove/roam/pull/244))
- Normalize initiator connector APIs ([#242](https://github.com/bearcove/roam/pull/242))
- Implement v9 transport prologue ([#241](https://github.com/bearcove/roam/pull/241))
- Implement automatic retry after session resume ([#239](https://github.com/bearcove/roam/pull/239))
- Add manual session resumption on a new conduit ([#237](https://github.com/bearcove/roam/pull/237))
- Implement retry operation identity core ([#236](https://github.com/bearcove/roam/pull/236))
- Define static retry policies ([#235](https://github.com/bearcove/roam/pull/235))
- Rewrite TypeScript runtime around layered sessions ([#233](https://github.com/bearcove/roam/pull/233))

### Changed

- Remove the implicit `From<DriverCaller> for ()` conversion and add `NoopCaller` for liveness-only root handles.

## [7.0.0-alpha.3](https://github.com/bearcove/roam/compare/roam-core-v7.0.0-alpha.2...roam-core-v7.0.0-alpha.3) - 2026-03-03

### Other

- Add MaybeSend bound on erased caller
