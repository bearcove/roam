# telex-types

Protocol and runtime data model shared across Telex implementations.

## Role in the Telex stack

`telex-types` spans the `Requests / Channels`, `Connections`, and `Session` layers by defining shared message and control types.

## What this crate provides

- Wire-level and runtime-facing enums/structs used by the protocol
- Request/response and channel-related types
- Common error and metadata types consumed by runtime and transports

## Fits with

- `telex`, `telex-core`, and transport crates (`telex-stream`, `telex-websocket`, `telex-shm`)
- `telex-codegen` when generating non-Rust bindings

Part of the Telex workspace: <https://github.com/bearcove/telex>
