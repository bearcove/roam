# telex-core

Core runtime for sessions, drivers, conduits, and connection orchestration.

## Role in the Telex stack

`telex-core` primarily implements the `Session`, `Connections`, and `Conduit` layers.

## What this crate provides

- Session builders (`session::initiator`, `session::acceptor`) and handles
- Driver runtime for dispatching inbound calls and issuing outbound calls
- Connection lifecycle primitives and runtime glue

## Fits with

- `telex` for service-facing APIs
- Link/transport crates (`telex-stream`, `telex-websocket`, `telex-shm`, `telex-local`)
- `telex-types` for protocol state and message types

Part of the Telex workspace: <https://github.com/bearcove/telex>
