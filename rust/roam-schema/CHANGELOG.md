# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0](https://github.com/bearcove/roam/releases/tag/roam-schema-v0.6.0) - 2026-01-16

### Fixed

- Codegen bugs for doc comments, enum types, and non-streaming services

### Other

- Add version requirements for workspace publish
- rename Tx/Rx, migrate codegen to modular structure, add wire protocol ([#4](https://github.com/bearcove/roam/pull/4))
- Split Stream into Push/Pull, rename Client/Server to Caller/Handler
- Set up captain hooks
- More renames, rapace => roam
