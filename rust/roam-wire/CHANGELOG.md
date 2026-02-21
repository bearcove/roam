# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-wire-v3.0.0...roam-wire-v4.0.0) - 2026-02-21

### Fixed

- restore Hello enum V1/V2 variants for wire compatibility

### Other

- Introduce ServiceDescriptor with precomputed RpcPlans ([#147](https://github.com/bearcove/roam/pull/147))
- Hello V6 protocol + diagnostic improvements ([#127](https://github.com/bearcove/roam/pull/127))
- Add V5 request concurrency flow control ([#101](https://github.com/bearcove/roam/pull/101))
- Remove COBS framing from protocol v4 ([#84](https://github.com/bearcove/roam/pull/84))
