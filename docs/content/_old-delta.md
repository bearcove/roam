# Spec Rewrite Tracking

This file tracks the status of the new spec relative to the old spec
(`_old-spec.md`). It records what's been ported, what's deliberately
excluded, and what's still pending.

## Ported (new spec has equivalent or better rules)

- Identifiers, Parity → `session.parity`, `connection.parity`
- Transports, Links → `spec/conn.md` link section
- Conduits → `spec/conn.md` conduit section
- Sessions, Handshake → `spec/conn.md` session section
- Virtual Connections → `connection.*` rules
- Service Definitions → `rpc.service`, `rpc.service.methods`
- Method Identity → `rpc.method-id.*` + `spec/sig.md`
- Signature Hash → `spec/sig.md` (full encoding spec)
- Schema Evolution → `rpc.schema-evolution`
- Calls (Request/Response) → `rpc.request`, `rpc.response`
- Error Scoping → `rpc.error.scope`
- RoamError → `rpc.fallible.roam-error`
- Channels (Tx/Rx) → `rpc.channel.*`
- Channel ID Allocation → `rpc.channel.allocation`
- Cancellation → `rpc.cancel`
- One service per connection → `rpc.one-service-per-connection` (NEW, not in old spec)
- Handler/Caller concepts → `rpc.handler`, `rpc.caller` (NEW)
- Session setup API → `rpc.session-setup` (NEW)
- Virtual connection accept/open → `rpc.virtual-connection.*` (NEW)

## Deliberately excluded or changed

- **Channel Ack/Seq** — old spec had per-channel Ack messages for exactly-once
  delivery and retransmission. Removed: reconnection is now a conduit-level
  concern (StableConduit), not per-channel.
- **Goodbye message** — replaced by `ProtocolError`. Same teardown semantics,
  different name.
- **Tx/Rx only in arguments** — old spec said channels MUST NOT appear in return
  types. New spec allows channels in return types (callee allocates those IDs).
  Channels still MUST NOT appear in the Err variant of Result return types.
- **Handler merging** — old spec (implicitly) supported combining multiple
  service handlers. New spec: one service per connection, use virtual connections
  for multiple services.
- **Service identity negotiation** — not added. Method IDs already encode the
  service name; unknown methods get per-call errors. No upfront service check.

## Ported (continued)

- Metadata → `rpc.metadata.*` in `spec/rpc.md` (flags, keys, duplicates, unknown handling; no size limits)
- Call pipelining → `rpc.pipelining` in `spec/rpc.md`
- Channel binding → `rpc.channel.discovery`, `rpc.channel.payload-encoding`, `rpc.channel.binding` in `spec/rpc.md`
- StableConduit → `spec/stable.md` (framing, seq/ack, handshake, resume, replay)
- Shared memory transport → `spec/shm.md` (BipBuffers, VarSlotPool, signaling, peer lifecycle)

## Deliberately excluded or changed (continued)

- **Idempotency / Nonces** — old spec had `core.nonce.*` for cross-session idempotency.
  Removed: StableConduit handles replay at the conduit level via seq/ack. Application-level
  idempotency is the application's concern.
- **Metadata size limits** — old spec enforced max 128 entries, 256-byte keys, 16KB values,
  64KB total. Removed: not found necessary.
- **SHM-specific message format** — old spec had ShmFrameHeader with msg_type, method_id, etc.
  Removed: SHM now uses the same postcard-encoded Message as other transports, with only a
  thin framing header (inline vs slot-ref) for delivery.
- **Per-channel flow control in SHM** — old spec had per-channel credit counters in the
  channel table. Deferred: flow control is a session-level concern to be specced separately.

## Still pending

- **Flow control** — backpressure for channels (open design question)
- **Topologies** — proxy patterns (old spec lines 395-405)
