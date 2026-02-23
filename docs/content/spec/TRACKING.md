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

## Still pending

- **Metadata** — keys, unknown key handling, flag semantics (old spec lines 624-720)
- **Idempotency** — idempotency keys, server dedup, client retry (old spec lines 300-393)
- **Call pipelining** — multiple requests in flight concurrently (old spec lines 842-856)
- **Flow control** — backpressure for channels (old spec lines 1783-1818)
- **StableConduit / Reconnection** — resume tokens, replay, reconnection (old spec lines 1866-1917)
- **Channel binding** — schema-driven channel discovery via facet walk (old spec lines 1918-1938)
- **Topologies** — proxy patterns (old spec lines 395-405)
- **Shared memory transport** — deserves its own spec document
