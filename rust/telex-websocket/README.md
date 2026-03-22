# telex-websocket

WebSocket transport binding for Telex links.

## Role in the Telex stack

`telex-websocket` implements the `Link` layer on top of WebSocket binary frames.

## What this crate provides

- Client/server link adapters over WebSocket transports
- Native and wasm-friendly runtime integration points

## Fits with

- `telex-core` for connection/session orchestration
- `telex-types` for protocol payloads and control messages
- `telex` for generated clients and service dispatchers

Part of the Telex workspace: <https://github.com/bearcove/telex>
