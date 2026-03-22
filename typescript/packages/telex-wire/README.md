# @bearcove/telex-wire

Wire-format types and codecs for the Telex protocol in TypeScript.

## Role in the Telex stack

`@bearcove/telex-wire` sits at the protocol boundary between runtime logic and transport links.

## What this package provides

- Message schemas and protocol type definitions
- Wire-level encode/decode helpers
- Shared wire constants and error representations

## Fits with

- `@bearcove/telex-postcard` for low-level serialization
- `@bearcove/telex-core` for runtime behavior on decoded messages
- Transport packages that carry encoded wire frames

Part of the Telex workspace: <https://github.com/bearcove/telex>
