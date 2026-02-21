# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-codegen-v0.6.0...roam-codegen-v4.0.0) - 2026-02-21

### Fixed

- *(codegen)* add type annotations to lambda parameters in encode expressions
- use namespace imports for postcard codegen
- proper error handling in TypeScript codegen
- *(codegen)* improve TypeScript doc comment generation

### Other

- Pass &'static MethodDescriptor through the entire call chain ([#148](https://github.com/bearcove/roam/pull/148))
- Fix Swift SHM channeling request framing (issue #120) ([#122](https://github.com/bearcove/roam/pull/122))
- Swift codegen: fix dispatcher naming and unused cursor emission ([#90](https://github.com/bearcove/roam/pull/90))
- Swift codegen: split client/server generation modes ([#89](https://github.com/bearcove/roam/pull/89))
- Swift runtime: add call timeouts + fail-fast disconnect handling ([#88](https://github.com/bearcove/roam/pull/88))
- Replace recursive walkers with precomputed RpcPlan, rename stream to channel ([#85](https://github.com/bearcove/roam/pull/85))
- Fix Swift codegen decode cursor name collisions ([#76](https://github.com/bearcove/roam/pull/76))
- Fix Swift enum encode/decode placeholders in codegen ([#70](https://github.com/bearcove/roam/pull/70))
- *(ts)* move encoding/decoding to Caller layer
- Add client-side middleware support for TypeScript clients ([#57](https://github.com/bearcove/roam/pull/57))
