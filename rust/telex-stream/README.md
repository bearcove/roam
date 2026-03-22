# telex-stream

Byte-stream transport binding for Telex over `AsyncRead`/`AsyncWrite`.

## Role in the Telex stack

`telex-stream` implements the `Link` layer using length-prefixed framing on stream transports.

## What this crate provides

- Framing and link adapters for TCP/Unix/stdio style byte streams
- Runtime-compatible transport glue for session establishment

## Fits with

- `telex-core` session and driver runtime
- `telex-local` for local IPC sockets and named pipes
- `telex-types` for transport message payloads

Part of the Telex workspace: <https://github.com/bearcove/telex>
