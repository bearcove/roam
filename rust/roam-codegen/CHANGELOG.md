# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-codegen-v0.6.0...roam-codegen-v4.0.0) - 2026-01-28

### Fixed

- *(codegen)* add type annotations to lambda parameters in encode expressions
- use namespace imports for postcard codegen
- proper error handling in TypeScript codegen
- *(codegen)* improve TypeScript doc comment generation

### Other

- *(ts)* move encoding/decoding to Caller layer
- Add client-side middleware support for TypeScript clients ([#57](https://github.com/bearcove/roam/pull/57))
