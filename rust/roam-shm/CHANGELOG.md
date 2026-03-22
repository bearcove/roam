# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/roam-shm-v7.3.0...roam-shm-v8.0.0) - 2026-03-22

### Other

- Purge live v7 naming from SHM surface
- Simplify channels greatly
- Split SchemaTracker into SchemaSendTracker + SchemaRecvTracker, inline schemas in RequestCall/RequestResponse
- migrate roam from facet-postcard to roam-postcard
- Align SHM with v9 transport prologue ([#246](https://github.com/bearcove/roam/pull/246))
