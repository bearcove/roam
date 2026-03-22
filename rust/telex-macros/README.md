# telex-service-macros

Procedural macros for generating Telex service clients and dispatchers.

## Role in the Telex stack

`telex-service-macros` powers the schema/source-of-truth layer where Rust traits define service contracts.

## What this crate provides

- `#[telex::service]` expansion support
- Generated service trait plumbing, client stubs, and dispatcher glue

## Fits with

- `telex` as the public API surface
- `telex-macros-core` and `telex-macros-parse` for expansion internals
- `telex-hash` for stable method identity

Part of the Telex workspace: <https://github.com/bearcove/telex>
