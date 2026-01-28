# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-stream-v0.6.0...roam-stream-v4.0.0) - 2026-01-28

### Added

- [**breaking**] wire protocol v3 with metadata flags ([#65](https://github.com/bearcove/roam/pull/65))
- *(session)* Add non-generic Caller methods to reduce monomorphization ([#61](https://github.com/bearcove/roam/pull/61))

### Fixed

- restore Hello enum V1/V2 variants for wire compatibility

### Other

- Optimize CobsFramed::send to avoid double allocation (fixes #59)
- shed ctor dependency, which isn't fuzzing friendly
- Minimize monomorphization with Shape-based dispatch ([#54](https://github.com/bearcove/roam/pull/54))
- Divide roam-session into several files, use decl_id properly
