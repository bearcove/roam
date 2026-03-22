# telex-macros-core

Core code generation engine for Telex service procedural macros.

## Role in the Telex stack

`telex-macros-core` is internal codegen infrastructure behind the service-definition layer.

## What this crate provides

- Macro expansion logic shared by public proc-macro entry points
- Token generation for clients, dispatchers, and service detail artifacts

## Fits with

- `telex-service-macros` (public proc-macro crate)
- `telex-macros-parse` (grammar/parser front-end)

Part of the Telex workspace: <https://github.com/bearcove/telex>
