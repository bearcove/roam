# Phase 010: Tunnel Streams (Tx/Rx<Vec<u8>> Replacement for rapace::TunnelStream)

## Goal

Support dodeca’s “TCP tunneling” use case (host accepts connections, cell
handles bytes) in the roam stack over roam-shm.

## Current State

- Dodeca uses `rapace::TunnelStream<AnyTransport>` (and friends) to tunnel bytes.
- Roam already supports streaming `Vec<u8>` payloads (`roam_wire::Value::Bytes(Vec<u8>)`)
  and stream handles (`roam_session::Tx<T>` / `roam_session::Rx<T>`), so “tunnel”
  can just be a pair of `Tx<Vec<u8>>`/`Rx<Vec<u8>>`.

## Target API

Proto (shared):

```rust
pub struct Tunnel {
    pub to_cell: roam_session::Tx<Vec<u8>>,
    pub from_cell: roam_session::Rx<Vec<u8>>,
}
```

Host:

```rust
let tunnel = http_cell.open_tunnel(/* ... */).await?;
tokio::spawn(async move { pump_tcp_to_cell(socket, tunnel.to_cell).await });
tokio::spawn(async move { pump_cell_to_tcp(tunnel.from_cell, socket).await });
```

## Design

Tunnel is just two roam streams of `Vec<u8>`:

- host → cell: `Tx<Vec<u8>>`
- cell → host: `Rx<Vec<u8>>`

Close/reset semantics come from normal stream close behavior (no separate tunnel
protocol needed).

## Implementation Plan

### 1. Define Tunnel Types in the Relevant Proto Crate

- Define a `Tunnel` struct (or equivalent) holding `Tx<Vec<u8>>` and `Rx<Vec<u8>>`.
- Add a roam service method that returns a `Tunnel` (or otherwise provisions the
  two streams).

### 2. Host Adapters

- Adapters:
  - `TcpStream -> Tx<Vec<u8>>` (read socket, chunk, send)
  - `Rx<Vec<u8>> -> TcpStream` (recv, write socket)
- Keep the implementation simple: a couple of tasks doing `read/write` + `send/recv`.

### 3. Cell Adapters

- In the cell, treat the tunnel as two async channels:
  - `Rx<Vec<u8>>` for inbound bytes
  - `Tx<Vec<u8>>` for outbound bytes

## Tasks

- [ ] Implement host pumps (`TcpStream` <-> `Tx/Rx<Vec<u8>>`)
- [ ] Implement cell pumps (`Tx/Rx<Vec<u8>>` <-> cell logic)
- [ ] Add tests (loopback + SHM once Phase 008 lands)

## Notes

- Keep chunk sizes bounded (e.g., 16–64KB) to avoid pathological slot usage.
- Do not block the driver task; tunnel pumps must run as independent tasks.
