# Phase 010: Tunnel Streams (Roam-Native Replacement for rapace::TunnelStream)

## Goal

Support dodeca’s “TCP tunneling” use case (host accepts connections, cell
handles bytes) in the roam stack over roam-shm.

## Current State

- Dodeca uses `rapace::TunnelStream<AnyTransport>` (and friends) to tunnel bytes.
- Roam currently has stream/message semantics, but no tunnel abstraction or
  helper runtime for “raw byte stream tunneled through RPC”.

## Target API

Host:

```rust
let (client, server) = roam_tunnel::open(/* ... */);
tokio::spawn(async move { pump_tcp_to_tunnel(socket, client).await });
```

Cell:

```rust
// Expose a service method that returns a tunnel endpoint.
// The cell consumes bytes and produces bytes via a bidirectional tunnel.
```

## Design

Prefer an explicit tunnel primitive implemented atop roam streams:

- Tunnel is a pair of streams:
  - host → cell bytes (Tx<Data>)
  - cell → host bytes (Tx<Data>)
- Add close/reset semantics mapped onto roam’s Close/Reset.

## Implementation Plan

### 1. Define Tunnel Protocol

- Add a `roam-tunnel` crate with:
  - a small record type for data chunks
  - helper to expose a bidirectional tunnel as a pair of stream endpoints

### 2. Host Adapters

- Adapters:
  - `TcpStream <-> TunnelEndpoint`
  - bounded buffering and backpressure behavior

### 3. Cell Adapters

- Provide a simple API for cell code:
  - accept a tunnel endpoint and treat it like an async `Read + Write`
  - or expose two async channels for bytes in/out

## Tasks

- [ ] Decide whether tunnel is “just streams” or a dedicated abstraction
- [ ] Implement host/cell adapters
- [ ] Add tests (loopback + SHM once Phase 008 lands)

## Notes

- Keep chunk sizes bounded (e.g., 16–64KB) to avoid pathological slot usage.
- Do not block the driver task; tunnel pumps must run as independent tasks.

