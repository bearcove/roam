# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.0](https://github.com/bearcove/roam/compare/roam-codegen-v7.3.0...roam-codegen-v8.0.0) - 2026-03-22

### Added

- implement TypeScript session resumption + fix RoamError schema via reflection
- comprehensive test coverage — 135/136 TypeScript tests pass
- TypeScript schema exchange — codegen-driven CBOR schemas for wire protocol

### Fixed

- fixme(garbage) helpers

### Other

- Clean up remaining Swift wire naming
- Rename Swift wire runtime types
- Clean up Swift subject coverage and TCP handshake paths
- Checkpoint Swift raw-link migration
- Start Swift canonical schema and inline channel migration
- Slim generated TypeScript service descriptors
- Clean up canonical TypeScript schema runtime
- Drop legacy TypeScript schema runtime paths
- Use canonical schemas for TS channel and response paths
- Align schema exchange and channel retry behavior
- Start TS canonical schema codec migration
- Outgoing/Incoming => Value/PostcardBytes
- Fix schema tracking scope and method root extraction
- Add args_type
- emit Schema objects as TypeScript literals instead of CBOR blobs
- TypeSchemaId => SchemaHash
- Remove RpcPlan/sidecar channel binding from driver and macros
- Bind channels during ser/deser via thread-local ChannelBinder
- Simplify channels greatly
- Remove const N generic from Tx/Rx, hardcode initial credit to 16
- Return root TypeRef from extract_schemas, resolve Vars in plan builder
- Deduplicate schemas by DeclId, emit generic type parameters
- Add CycleSchemaIndex and TypeParamName newtypes, align spec
- Newtypes are cool
- SchemaHasher context + generic TypeRef + no naked u64s
- typestate for schema IDs: MixedId during extraction, TypeSchemaId after
- use finalize_content_hashes for constructed schemas
- align TypeScript schema types with spec and Rust changes
- implement blake3 content hashing for TypeSchemaId
- align schema types between spec and Rust implementation
- phase 3: opaque u32le payload framing — fix codegen, schema, codec, golden vectors
- merge types/schemas into single wire.generated.ts, auto-derive everything from Message shape
- auto-derive discriminants and narrowed aliases, remove stale Hello/HelloYourself from wire schemas
- Refine retry semantics
- WIP backwards compat
- Generate TypeScript wire protocol types from Rust shapes
- Add channel retry semantics matrix coverage ([#251](https://github.com/bearcove/roam/pull/251))
- Add Swift retry semantics plumbing ([#249](https://github.com/bearcove/roam/pull/249))
- Normalize initiator connector APIs ([#242](https://github.com/bearcove/roam/pull/242))
- Implement automatic retry after session resume ([#239](https://github.com/bearcove/roam/pull/239))
- Define static retry policies ([#235](https://github.com/bearcove/roam/pull/235))
- Rewrite TypeScript runtime around layered sessions ([#233](https://github.com/bearcove/roam/pull/233))
