# @bearcove/telex-core

Core TypeScript runtime abstractions for Telex connections, calls, and channeling.

## Role in the Telex stack

`@bearcove/telex-core` implements TypeScript-side runtime behavior at the `Requests / Channels`, `Connections`, and `Session` layers.

## What this package provides

- Caller/connection abstractions for generated and hand-written clients
- Call-building and middleware-style runtime plumbing
- Channeling primitives used by higher-level transports and generated bindings

## Fits with

- `@bearcove/telex-wire` for wire message types/codecs
- `@bearcove/telex-postcard` for serialization
- `@bearcove/telex-tcp` and `@bearcove/telex-ws` for concrete transports

Part of the Telex workspace: <https://github.com/bearcove/telex>
