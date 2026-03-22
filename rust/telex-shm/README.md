# telex-shm

Shared-memory transport implementation for Telex.

## Role in the Telex stack

`telex-shm` implements the `Link` layer for zero-copy local IPC using shared-memory rings.

## What this crate provides

- Host/guest shared-memory link construction
- Segment and peer orchestration helpers
- Framing and runtime integration for SHM-backed connections

## Fits with

- `shm-primitives` and `shm-primitives-async` for low-level memory/control operations
- `telex-core` for session, connection, and driver orchestration on top of SHM links

Part of the Telex workspace: <https://github.com/bearcove/telex>
