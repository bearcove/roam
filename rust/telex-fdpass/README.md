# telex-fdpass

Cross-platform descriptor/handle passing primitives for local IPC scenarios.

## Role in the Telex stack

`telex-fdpass` is a low-level transport helper used below `Link` setup for passing OS resources between peers.

## What this crate provides

- Unix FD passing support
- Windows handle passing support
- Utilities to integrate descriptor passing into local transport bootstrapping

## Fits with

- `telex-local` and stream-based local connection setup
- `telex-shm` bootstrap paths that may require OS resource transfer

Part of the Telex workspace: <https://github.com/bearcove/telex>
