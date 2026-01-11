# Phase 009: Tracing Across Cells (Roam-Native Replacement for rapace_tracing)

## Goal

Replace the `rapace_tracing` + `rapace_cell` tracing plumbing used by dodeca
with a roam-native tracing story that works over roam-shm.

Minimum target: cells can emit tracing events/spans that the host can collect
and present (or forward to a subscriber), with backpressure and bounded memory.

## Current State

- Dodeca uses `rapace_tracing` service(s) and `rapace_cell` session helpers.
- Roam does not currently provide an equivalent tracing service/runtime crate.
- roam-shm provides transport primitives but not a tracing protocol.

## Target API

Host side:

```rust
// Host installs a collector and optionally pushes config to the cell.
let tracing = roam_tracing::TracingHost::new(/* ... */);
tracing.register_peer(peer_id, &handle);
```

Cell side:

```rust
// Cell installs a layer that forwards to host.
roam_tracing::init_cell_tracing(/* ... */);
```

## Design

### Protocol Shape

Prefer a simple service pair:

- `TracingConfig` (host → cell): configure level/filters, sampling knobs, etc.
- `TracingSink` (cell → host): send event/span records as a stream.

Important constraints:
- Must be **lossy or backpressured** by design (bounded queue).
- Must not deadlock the RPC runtime (sending tracing should not require making
  new blocking RPC calls on the same saturated connection).

### Transport

Use roam’s stream primitives (Tx/Rx) to deliver tracing records, but ensure the
implementation works over SHM once Phase 008 exists.

## Implementation Plan

### 1. Define Schema / Services

- Add a `roam-tracing` crate (or `roam-tracing-proto` + runtime crate) with:
  - `#[roam::service]` traits for config + sink
  - a compact record format (avoid huge allocations)

### 2. Host Collector

- Implement a host-side sink server that:
  - accepts a per-cell stream of tracing records
  - tags events with `peer_id` and (optional) `peer_name`
  - forwards into `tracing` as structured events or stores for inspection

### 3. Cell Forwarder Layer

- Implement a tracing layer/subscriber for the cell that:
  - serializes events/spans into records
  - enqueues into a bounded channel
  - has a background task that drains to the host over a stream

### 4. Integration With Spawn + Lifecycle

- Add glue so dodeca can:
  - install host collector when spawning a cell
  - optionally send config updates to running cells

## Tasks

- [ ] Decide crate layout (`roam-tracing` vs `roam-tracing-proto`)
- [ ] Define record format (events + spans + fields)
- [ ] Implement host sink server + optional config client
- [ ] Implement cell forwarding layer + bounded buffering behavior
- [ ] Add tests (in-process + over SHM once Phase 008 lands)

## Notes

- Keep this independent of any specific UI (TUI/web); the host should expose a
  minimal API to subscribe/consume records.
- If we need “crash last N logs”, prefer a ring buffer in host memory.

