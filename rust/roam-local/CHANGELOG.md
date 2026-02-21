# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-local-v0.6.0...roam-local-v4.0.0) - 2026-02-21

### Other

- migrate roam integration to moire dependency names
- migrate deprecated peeps fn calls to macros
- Remove redundant peepable wrappers from first-class resources
- Migrate roam-local windows pipe retry sleep to peeps::sleep
- More peeps instrumentation
- Improve .peepable() labels for clarity and uniqueness
- instrument socket waits with peepable futures
- Minimize monomorphization with Shape-based dispatch ([#54](https://github.com/bearcove/roam/pull/54))
