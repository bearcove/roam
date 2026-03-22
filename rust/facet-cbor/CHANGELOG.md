# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/facet-cbor-v7.3.0...facet-cbor-v8.0.0) - 2026-03-22

### Added

- add public from_slice API and wire up decode/deserialize modules
- implement CBOR deserialization via Partial API
- add UnexpectedEof, InvalidCbor, TypeMismatch error variants
- add CBOR decoding primitives for deserialization
- implement CBOR encoding primitives and Peek-based serialization
- add facet-cbor crate skeleton with workspace integration

### Fixed

- add Array and transparent type deserialization to facet-cbor
- add #[repr(u8)] to test enum required by Facet derive
- check Def::Option before UserType::Enum in serialization dispatch

### Other

- Add internally-tagged enum support to facet-cbor and unify schema wire format
- Fix all clippy warnings and errors across workspace
- Take care of warnings a bit
- add comprehensive CBOR deserialization round-trip tests
- add comprehensive CBOR serialization tests
