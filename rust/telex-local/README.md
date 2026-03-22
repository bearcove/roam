# telex-local

Cross-platform local IPC transport utilities for Telex.

## Role in the Telex stack

`telex-local` helps construct `Link`-layer connections for same-host communication.

## What this crate provides

- Unix domain socket support on Unix targets
- Named pipe support on Windows targets
- Local transport setup used by higher-level stream integration

## Fits with

- `telex-stream` framing and link adaptation
- `telex-core` session establishment and driver runtime

Part of the Telex workspace: <https://github.com/bearcove/telex>
