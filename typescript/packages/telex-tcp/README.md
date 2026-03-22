# @bearcove/telex-tcp

TCP transport binding for Telex in TypeScript.

## Role in the Telex stack

`@bearcove/telex-tcp` implements the `Link` layer over Node.js TCP streams.

## What this package provides

- Framing and transport adapter logic for TCP links
- Integration with `@bearcove/telex-core` connection/runtime abstractions

## Fits with

- `@bearcove/telex-core` for session/call orchestration
- `@bearcove/telex-wire` and `@bearcove/telex-postcard` for protocol payloads

Part of the Telex workspace: <https://github.com/bearcove/telex>
