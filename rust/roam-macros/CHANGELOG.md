# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-service-macros-v0.6.0...roam-service-macros-v4.0.0) - 2026-01-28

### Added

- improve RPC error observability

### Other

- Upgrade facet-cargo-toml
- Add per-argument span attributes via Context.arg_names ([#56](https://github.com/bearcove/roam/pull/56))
- Add roam-telemetry crate and pre/post middleware support ([#55](https://github.com/bearcove/roam/pull/55))
- Minimize monomorphization with Shape-based dispatch ([#54](https://github.com/bearcove/roam/pull/54))
- Replace Never shim with std::convert::Infallible
