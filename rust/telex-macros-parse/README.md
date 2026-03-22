# telex-macros-parse

Parser and grammar utilities for Telex service macro inputs.

## Role in the Telex stack

`telex-macros-parse` supports the compile-time schema layer by parsing service trait definitions.

## What this crate provides

- Parser structures for service-trait syntax and macro input handling
- Intermediate representations consumed by macro codegen

## Fits with

- `telex-macros-core` for expansion/token generation
- `telex-service-macros` as the public macro crate

Part of the Telex workspace: <https://github.com/bearcove/telex>
