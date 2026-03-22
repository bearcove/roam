# telex-codegen

Language binding generator for Telex service descriptors.

## Role in the Telex stack

`telex-codegen` bridges Rust-defined schemas to non-Rust clients/servers above the RPC surface.

## What this crate provides

- TypeScript and Swift code generation targets
- Rendering of service descriptors into client/server scaffolding

## Fits with

- `telex` service definitions and generated descriptors
- `telex-hash` and `telex-types` for shared protocol identity and type model

Part of the Telex workspace: <https://github.com/bearcove/telex>
