# @bearcove/telex-ws

WebSocket transport binding for Telex in TypeScript.

## Role in the Telex stack

`@bearcove/telex-ws` implements the `Link` layer over WebSocket binary frames.

## What this package provides

- WebSocket transport adapters with reconnecting utilities
- Integration with `@bearcove/telex-core` runtime abstractions

## Fits with

- `@bearcove/telex-core` for connection/session runtime behavior
- `@bearcove/telex-tcp` where shared transport logic is reused
- `@bearcove/telex-wire` for protocol message payloads

Part of the Telex workspace: <https://github.com/bearcove/telex>
