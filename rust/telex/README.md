# telex

High-level Rust API for defining, implementing, and consuming Telex services.

## Role in the Telex stack

`telex` sits at the RPC surface (`Requests / Channels`) and exposes the developer-facing service model.

## What this crate provides

- `#[telex::service]`-driven service definitions and generated clients/dispatchers
- Core RPC traits and types re-exported for app-level use
- Integration point for the rest of the Rust runtime crates

## Fits with

- `telex-core` for session/driver/runtime internals
- `telex-types` for protocol data model
- `telex-service-macros` for code generation from service traits

Part of the Telex workspace: <https://github.com/bearcove/telex>
