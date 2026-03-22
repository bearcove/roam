# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/roam-schema-v0.6.0...roam-schema-v8.0.0) - 2026-03-22

### Added

- wire schema-aware deserialization with translation plans
- add roam-postcard crate with translation plan support
- add build_schema_message/parse_schema_message for CBOR wire encoding
- add SchemaTracker for per-session schema exchange state tracking
- implement extract_schemas to walk Shape tree and build Schema structs
- add type_id_of function to compute TypeId from Shape via blake3
- define schema wire types — TypeId, Schema, SchemaKind, FieldSchema, VariantSchema, VariantPayload, PrimitiveType, SchemaMessage
- add roam-schema crate manifest with facet dependencies
- [**breaking**] Roam v7 rewrite (spec-first, major delta from legacy) ([#168](https://github.com/bearcove/roam/pull/168))

### Fixed

- fixme(garbage) helpers
- add #[repr(u8)] to all enums as required by Facet derive

### Other

- Split schema model out of roam-types
- align schema types between spec and Rust implementation
- Merge roam-schema + roam-schema-extract into roam-types
- Fix all clippy warnings and errors across workspace
- extract roam-schema-extract crate, fix sentinel passthrough detection
- Add tracey annotations for translation plan and error spec rules
- Add tracey annotations for schema exchange spec coverage
- add SchemaTracker and CBOR round-trip tests
- add facet-cbor dependency to roam-schema for CBOR schema serialization
- add comprehensive tests for extract_schemas and type_id_of
- add roam-types, roam-hash, and blake3 dependencies to roam-schema
- add roam-schema to workspace members and dependencies
- Pass &'static MethodDescriptor through the entire call chain ([#148](https://github.com/bearcove/roam/pull/148))
- Divide roam-session into several files, use decl_id properly
