# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/roam-postcard-v7.3.0...roam-postcard-v8.0.0) - 2026-03-22

### Added

- wire schema-aware deserialization with translation plans
- add ScatterPlan for zero-copy serialization
- add opaque adapter passthrough to roam-postcard
- add borrowed deserialization and in-place API to roam-postcard
- rich error reporting for translation plan building
- add roam-postcard crate with translation plan support

### Fixed

- fixme(garbage) helpers
- double-free in proxy deser, bump moire to drop ctor from disabled path
- add Def::Result, proxy support to roam-postcard; restore zero-copy scatter
- trailing-aware opaque serialization in roam-postcard

### Other

- Split schema model out of roam-types
- Use canonical schemas for TS channel and response paths
- Align schema exchange and channel retry behavior
- Align byte-buffer schema compatibility across Rust and TS
- Fix inline channel binding on send
- Add internally-tagged enum support to facet-cbor and unify schema wire format
- TypeSchemaId => SchemaHash
- TranslationPlan redo
- Simplify channels greatly
- Fix BORROW flag ignored for Str and CowStr deserialization
- Add Cow, integer, slice tests; fix round_trip to use HRTB + internal assert
- Add failing memory safety tests: BORROW=false allows borrowed references
- Add failing tests: translation plans don't recurse into containers
- Add error Display coverage tests for all error variants
- Use from_slice_borrowed in round_trip helper, simplify borrowed tests
- Add borrowed slice/str tests with small and large payloads
- Parameterize round-trip tests to exercise both direct and scatter paths
- Add proxy deserialization tests, fix UnresolvedVar error in plan builder
- Add coverage tests for f32/i128 skip, truncated input errors, f32 round-trip
- Add coverage tests for enum tuple skip, char, unit, u128, borrowed plan, HashSet
- Add coverage tests for container types, 128-bit integers, transparent wrappers
- Add coverage tests for skip_value, tuple plans, name mismatch, deserialize_into
- Return root TypeRef from extract_schemas, resolve Vars in plan builder
- Deduplicate schemas by DeclId, emit generic type parameters
- Add SchemaKind::Channel with direction, element type, and initial credit
- Add CycleSchemaIndex and TypeParamName newtypes, align spec
- SchemaHasher context + generic TypeRef + no naked u64s
- align schema types between spec and Rust implementation
- Add 4K scatter reference threshold: small blobs staged, large blobs zero-copy
- Optimize ScatterBuilder: lazy staged segment flushing
- Add test proving staged segment coalescence in scatter plan
- Fix &[u8] slice serialization: zero-copy via write_referenced_bytes
- Use write_referenced_bytes for passthrough opaque payload (zero-copy)
- Use u32le length prefix for opaque values, eliminate temp Vec allocation
- Fix incorrect comment claiming pointer identity for Shape comparison
- Add MaybeSendFuture trait, BoxFut/CallResult aliases, box TranslationErrorKind
- Fix clippy warnings: collapsible ifs, redundant closures, type aliases, approx constants
- Schema validation: type name checks, SchemaPath newtype, rich error types
- Remove #[facet(trailing)] — all opaque fields are length-prefixed
- Fix test type inference and clean up debug prints
- Schema-vs-schema plan building, type names in schemas
- Fix enum variant field deserialization in plan-based deserializer
- Merge roam-schema + roam-schema-extract into roam-types
- WIP backwards compat
- Fix all clippy warnings and errors across workspace
- Do scatter planning properly
- extract roam-schema-extract crate, fix sentinel passthrough detection
- fix opaque adapter serialization — inline non-passthrough values
- migrate roam from facet-postcard to roam-postcard
- Add tracey annotations for translation plan and error spec rules
- add translation plan tests for roam-postcard
