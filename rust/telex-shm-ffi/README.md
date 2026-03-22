# telex-shm-ffi

C-compatible bindings for Telex shared-memory primitives.

## Role in the Telex stack

`telex-shm-ffi` exposes SHM building blocks at the FFI boundary so non-Rust runtimes can interoperate.

## What this crate provides

- C ABI wrappers around shared-memory primitive operations
- Headers/artifacts for embedding SHM support in foreign runtimes

## Fits with

- `shm-primitives` and `telex-shm` internals
- Swift and other non-Rust integrations that need SHM access

Part of the Telex workspace: <https://github.com/bearcove/telex>
