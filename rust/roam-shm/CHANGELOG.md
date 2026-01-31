# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-shm-v0.6.0...roam-shm-v4.0.0) - 2026-01-31

### Added

- [**breaking**] wire protocol v3 with metadata flags ([#65](https://github.com/bearcove/roam/pull/65))
- *(shm)* Implement backpressure queue limits and Clean Abort strategy

### Other

- Minimize monomorphization with Shape-based dispatch ([#54](https://github.com/bearcove/roam/pull/54))
- Divide roam-session into several files, use decl_id properly
