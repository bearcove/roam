+++
title = "Retry"
description = "Retry safety and semantics in roam"
weight = 13
+++

> r[retry]
>
> The retry layer defines how roam handles ambiguous failures — cases where the
> client does not know whether the server received, started, or completed a
> request. It sits above the transport and session layers and below application
> logic.

# The fundamental ambiguity

> r[retry.ambiguity]
>
> After any communication failure, the client faces irreducible uncertainty.
> The previous attempt is in one of these conditions, and the client cannot
> distinguish them:
>
>   1. The request never left the client's outbound buffer
>   2. The request arrived but the handler never started
>   3. The handler started and is still running
>   4. The handler completed but the response was lost in transit
>   5. The handler started and then failed, with or without partial side effects
>
> Any retry mechanism MUST handle all five as possible realities behind a
> single "unknown" from the client's perspective.

# Operation identity

> r[retry.op-id]
>
> Every RPC is bound to an **operation ID** — a client-generated identifier
> that names the client's intention to cause exactly one execution of a
> particular method with particular arguments, yielding exactly one outcome.

> r[retry.op-id.uniqueness]
>
> The client MUST mint a unique operation ID for each logical operation. Every
> delivery attempt for that operation carries the same ID. A new intention —
> even with identical arguments — gets a new ID.

> r[retry.op-id.scope]
>
> Operation IDs are scoped to a session. When a session ends cleanly, all
> operation records for that session may be evicted.

> r[retry.op-id.payload-binding]
>
> If the same operation ID arrives with a different method or different
> serialized arguments, the server MUST reject it as a conflict. An operation
> ID binds method identity and request payload; changing either requires a
> new operation ID.

# Operation state machine

> r[retry.state]
>
> The server maintains a record mapping operation IDs to states. A logical
> operation proceeds through this lifecycle:
>
> ```
>               ┌──────────────┐
>               │   Absent     │  ← no record exists
>               └──────┬───────┘
>                      │ first attempt arrives
>                      ▼
>               ┌──────────────┐
>          ┌────│    Live      │────┐
>          │    └──────┬───────┘    │
>          │           │            │
>    abort/cancel      │ handler    │ crash or cancel
>    (pre-commit)      │ seals      │ (post-commit,
>          │           │ outcome    │  outcome lost)
>          ▼           ▼            ▼
>   ┌──────────┐  ┌──────────┐  ┌──────────────┐
>   │ Released │  │ Sealed   │  │Indeterminate │
>   │(→ Absent)│  │(outcome) │  │              │
>   └──────────┘  └──────────┘  └──────────────┘
> ```

> r[retry.state.absent]
>
> **Absent.** No record of this operation ID. A new attempt triggers normal
> handler dispatch and transitions to Live.

> r[retry.state.live]
>
> **Live.** The handler is currently executing. The operation has not yet
> committed an outcome. There is at most one live handler execution for
> any given operation ID.

> r[retry.state.sealed]
>
> **Sealed(outcome).** A terminal outcome — success or failure — has been
> recorded. This is the absorbing state: once sealed, an operation MUST
> NOT be unsealed by cancellation or any other event. Retries replay
> the sealed outcome.

> r[retry.state.released]
>
> **Released.** The operation was aborted before committing, and the server
> has confirmed no side effects leaked. The operation ID effectively returns
> to Absent. A subsequent attempt gets a fresh execution.

> r[retry.state.indeterminate]
>
> **Indeterminate.** The server crashed or lost state while the operation was
> Live. The outcome is genuinely unknown. How this resolves depends on the
> method's retry category (see `r[retry.category]`).

# Duplicate attempt handling

> r[retry.duplicate]
>
> When a retry arrives with an operation ID that is already known to the
> server, behavior depends on the operation's current state.

> r[retry.duplicate.absent]
>
> If Absent: admit the attempt and start the handler. Transition to Live.

> r[retry.duplicate.live]
>
> If Live: do NOT start a second handler. The duplicate attempt MUST attach
> to the existing in-progress operation and wait for the same outcome. When
> the handler finishes, all attached attempts receive the result.

> r[retry.duplicate.sealed]
>
> If Sealed: replay the cached terminal outcome. The handler MUST NOT be
> re-invoked.

> r[retry.duplicate.released]
>
> If Released: the operation ID has returned to Absent. The duplicate is
> treated as a fresh first attempt.

> r[retry.duplicate.expired]
>
> If the operation record has been evicted but the server can recognize the
> ID as expired (see `r[retry.gc.fail-closed]`), the server MUST reject
> the retry with an explicit "expired" error. It MUST NOT treat an expired
> ID as Absent.

# Method categories

> r[retry.category]
>
> Every service method MUST be assigned one of three retry categories. The
> category determines what guarantees the runtime can provide and what
> obligations the handler must satisfy.

## Rerunnable

The simplest category. A rerunnable method is one where running it again is
always fine — same inputs, same observable result, no matter how many times
you do it. Think of it as: "if in doubt, just do it again."

**Examples:**

- `get_user(user_id)` — reading data is inherently repeatable
- `set_temperature(thermostat_id, 72.0)` — setting a value to a specific
  target is the same whether you do it once or five times
- `upsert_config(key, value)` — "insert or update" by key converges to the
  same state regardless of repetition
- `compute_hash(data)` — pure computation, no side effects at all

The runtime doesn't need to do anything special here. No operation log, no
deduplication machinery. If a response gets lost and the client retries,
the server can just run the handler again. The only reason to cache results
for rerunnable methods is performance, not correctness.

> r[retry.category.rerunnable]
>
> The handler is naturally idempotent: executing it N times with the same
> arguments produces the same observable effects as executing it once. Reads,
> upserts-by-key, pure computations, and "set X to V" operations fall here.

> r[retry.category.rerunnable.contract]
>
> **Handler obligation:** the handler MUST tolerate being run more than once
> for the same logical inputs. No additional cooperation with the runtime is
> required.

> r[retry.category.rerunnable.behavior]
>
> The runtime MAY re-execute the handler (always safe) or return a cached
> result if one exists (an optimization). The runtime does not need to
> maintain operation state for correctness, only for efficiency.

> r[retry.category.rerunnable.indeterminate]
>
> On Indeterminate state (crash recovery): re-execute. Safe by definition.

## Deduplicable

This is the most interesting category, and the one that does the most heavy
lifting. A deduplicable method is NOT safe to run twice on its own — but
the runtime can make it safe by tracking the operation ID and caching the
outcome.

The trick: the handler doesn't need to be idempotent *everywhere*. It only
needs to be idempotent in the window before it commits. After it seals its
outcome, the runtime takes over and replays the cached result on any retry.
The "idempotency" comes from the pair of (handler + runtime deduplication),
not from the handler alone.

**Examples:**

- `transfer_money(from, to, amount)` — running this twice would move money
  twice. But if the handler writes the balance changes and seals the
  operation ID in the same database transaction, crash recovery is clean:
  either the transaction committed (replay the result) or it didn't
  (re-execute safely, because the balances weren't touched).

- `create_order(customer_id, items)` — each execution creates a new order.
  But if the handler wraps order creation + seal in one transaction, the
  runtime guarantees exactly one order per operation ID. If the response is
  lost, the client retries and gets back the same order ID without creating
  a duplicate.

- `reserve_seat(event_id, seat_number)` — double-reserving the same seat
  would be a bug. The handler claims the seat and seals in one atomic step.
  If the client retries, it gets the original reservation confirmation.

The critical obligation on the handler is **atomicity**: if it crashes or
fails before sealing, any partial work must either be rolled back (e.g., by
the database transaction aborting) or be arranged so that re-execution will
fix things up. If the handler can't make that promise, it doesn't belong in
this category.

> r[retry.category.deduplicable]
>
> The handler is NOT safe to run twice, but the runtime can enforce at-most-once
> execution by tracking the operation ID and caching the outcome.

> r[retry.category.deduplicable.contract]
>
> **Handler obligation — atomicity:** if the handler fails or is interrupted
> before it explicitly seals its outcome, any side effects it produced MUST
> be either rolled back or arranged so that a fresh re-execution from the
> beginning will subsume or correct them. Pre-commit failure MUST leave the
> world in a state where re-execution is safe.

> r[retry.category.deduplicable.behavior]
>
> System behavior on duplicate:
>
>   * If Live: attach the duplicate, wait for the outcome, return it
>   * If Sealed: return the cached outcome
>   * If Absent: execute normally
>
> After the handler seals its outcome, re-execution MUST NOT occur — the
> runtime replays the cached result.

> r[retry.category.deduplicable.indeterminate]
>
> On Indeterminate state: the atomicity obligation is load-bearing. If the
> handler's effects and the operation record are committed together (e.g., same
> transaction), then on recovery the system can inspect durable state: either
> the seal is present (replay it) or it is not (effects did not persist,
> re-execute safely).

> r[retry.category.deduplicable.durability]
>
> For deduplicable methods, the seal MUST be durable. An in-memory-only seal
> is not a real seal — a crash would lose it and violate at-most-once.

## Non-retryable

Sometimes you just can't make retry safe, and the honest thing is to say so.
A non-retryable method touches systems that don't participate in the
operation identity discipline — there's no way to atomically commit both
the side effect and the seal, so after an ambiguous failure, nobody knows
what happened.

**Examples:**

- `send_webhook(url, payload)` — the handler POSTs to a third-party API
  that has no idempotency key support. If the POST goes through but the
  server crashes before sealing, the operation is Indeterminate. Re-executing
  would send the webhook again — maybe that's a duplicate Slack notification,
  maybe it's a duplicate payment trigger. The runtime can't know.

- `send_sms(phone_number, message)` — the SMS is gone the moment the
  carrier accepts it. If the server crashes after sending but before
  sealing, there's no transaction to roll back. The message was sent.

- `trigger_deploy(service, version)` — kicks off a deploy pipeline in an
  external CI system. Once the pipeline starts, there's no "undo" and no
  way to check atomically whether it was triggered.

For these methods, the system is honest about the gap. If a retry hits an
Indeterminate state, it doesn't optimistically re-execute — it tells the
client "the outcome is unknown, investigate through other means." The
caller should have a separate read method (like `get_webhook_delivery_status`
or `get_deploy_status`) to figure out what actually happened.

Note that non-retryable methods still benefit from the operation state
machine in the non-crash cases: if the response was simply lost (server
sealed successfully, response didn't make it back), a retry replays the
sealed outcome. The difference from deduplicable only shows up in the
Indeterminate case — the worst case.

> r[retry.category.non-retryable]
>
> The handler cannot support transparent retry. Its effects are not naturally
> idempotent, and it cannot satisfy the atomicity obligation required for
> deduplication — perhaps because it touches external systems without their
> own idempotency support, or because partial effects are irrecoverable.

> r[retry.category.non-retryable.contract]
>
> **Handler obligation:** acknowledge that the runtime cannot guarantee
> exactly-once execution. The method SHOULD be designed so that callers can
> query for the outcome through a separate channel if needed (e.g., a read
> method that checks whether the effect happened).

> r[retry.category.non-retryable.behavior]
>
> System behavior on duplicate:
>
>   * If Sealed: return the cached outcome (still safe and useful)
>   * If Live: attach and wait, same as deduplicable
>   * If Absent: execute normally
>   * If Indeterminate: REJECT the duplicate with a clear error indicating
>     that the outcome is unknown and cannot be safely retried. The client
>     MUST investigate through other means or issue a new logical operation

> r[retry.category.non-retryable.distinction]
>
> The difference from deduplicable is entirely about the Indeterminate case.
> A non-retryable method is honest that re-execution after ambiguous failure
> is not safe.

# Sealing

> r[retry.seal]
>
> The runtime provides an explicit seal/commit operation in the handler API.
> The seal marks the point of no return for a logical operation.

> r[retry.seal.implicit]
>
> Returning a success value from a handler implicitly seals the operation.

> r[retry.seal.explicit]
>
> A handler MAY explicitly seal before returning — for example, to record
> that irreversible work has been done and the answer is an error
> ("seal-then-fail"). This MUST be expressible in the handler API.

> r[retry.seal.post-commit-failure]
>
> If the handler committed side effects and then reports failure, that failure
> is the true outcome of the operation. Replaying it on retry is correct —
> re-executing would attempt to repeat the committed effects.

> r[retry.seal.pre-commit-failure]
>
> If the handler failed before sealing — before any durable side effects —
> the operation SHOULD be released. This allows the client to retry and get
> a fresh execution. This is the common case for transient errors (downstream
> timeout, temporary resource exhaustion).

> r[retry.seal.terminal-replay]
>
> A sealed failure MUST be replayed on retry, not optimistically retried.
> A retry of the same logical operation MUST NOT turn a sealed validation
> error into success. The meaning of "same operation" requires this.

> r[retry.seal.absorbing]
>
> Once an operation is sealed, no subsequent event (cancellation, disconnect,
> crash recovery) can unseal it. Sealed is absorbing.

# Cancellation interaction

> r[retry.cancel]
>
> Cancellation interacts with the operation state machine. There are two
> distinct events that look like cancellation.

> r[retry.cancel.explicit]
>
> **Explicit cancellation.** The client actively requests that the operation
> be aborted. If the operation is pre-commit, the handler SHOULD be aborted
> and the operation released. If the operation is post-commit (sealed),
> cancellation is a no-op — the outcome is sealed and the cancel arrives
> too late.

> r[retry.cancel.implicit]
>
> **Implicit cancellation (lost interest).** The client disconnected or
> stopped waiting. The client might come back. Behavior depends on
> the method category:
>
>   * **Rerunnable:** the server MAY abort the handler (saves resources),
>     because re-execution on reconnect is safe
>   * **Deduplicable:** the server SHOULD continue the handler to
>     completion and cache the outcome, so that when the client retries,
>     the answer is waiting
>   * **Non-retryable:** the server's choice; either way, the operation
>     state MUST accurately reflect what happened

> r[retry.cancel.race]
>
> Cancellation competes with commit. Whichever transition (cancel-and-release
> vs. seal) reaches the operation record first wins. The client MUST be
> prepared for either outcome.

> r[retry.cancel.retry-after]
>
> A retry with the same operation ID after a cancel request MUST reattach to
> the same operation, not create a new one. If the operation seals as
> cancelled, retries replay the cancelled outcome. To try again from scratch,
> the client MUST use a new operation ID.

> r[retry.cancel.sealed-immutable]
>
> Cancellation MUST NOT unseal an operation. If the handler committed before
> the cancellation was processed, the committed outcome stands.

# Attempt failure vs. operation outcome

> r[retry.attempt-vs-outcome]
>
> Attempt failures and operation outcomes are distinct concepts.

> r[retry.attempt-vs-outcome.attempt]
>
> **Attempt failures** are failures of a particular delivery/execution attempt:
> connection dropped, timeout waiting for response, process died before durable
> seal. These are NOT automatically operation outcomes.

> r[retry.attempt-vs-outcome.operation]
>
> **Operation outcomes** are the final outcomes of the logical operation:
> success, business rejection, terminal failure. Transparent retry is defined
> over operations, not over attempts.

> r[retry.attempt-vs-outcome.seal-rule]
>
> A failure seals the operation only if the system has chosen it as the
> authoritative terminal outcome of the logical operation. A transient
> pre-commit crash does not seal. A validation error seals. A post-commit
> failure seals.

# Transport vs. operation layer

> r[retry.layers]
>
> The transport and operation layers have distinct responsibilities regarding
> retry.

> r[retry.layers.transport]
>
> The transport layer's job is to deliver bytes, manage connections, handle
> reconnection, and provide flow control. If the transport knows a message
> was never transmitted (still in the send buffer when the connection dropped),
> it MAY retransmit transparently — this is below the operation layer's concern.

> r[retry.layers.boundary]
>
> If the transport does NOT know whether a message reached the server, it
> MUST surface this uncertainty to the operation layer rather than silently
> resending. The transport MUST NOT silently retry operations.

> r[retry.layers.reconnect]
>
> After a reconnect, the session layer MUST report which in-flight operations
> have ambiguous delivery status. The operation layer then re-sends those as
> explicit retry attempts with the original operation IDs.

> r[retry.layers.reattach]
>
> If the session layer can resume a connection and the server confirms that
> a particular in-flight operation is still Live, the session layer MAY
> wait for the result over the new connection without re-sending the request.

# Operation record lifetime

> r[retry.gc]
>
> The server cannot keep operation records forever, but premature eviction
> is dangerous.

> r[retry.gc.ttl]
>
> Operation records MUST have a TTL that exceeds the maximum retry window
> by a comfortable margin. TTL countdown MUST start only after the operation
> reaches a terminal state, not from request arrival. Live operations MUST
> NOT be evicted while the handler is alive.

> r[retry.gc.session-scoped]
>
> Session-scoped operation IDs simplify lifetime management: when a session
> ends cleanly, all its operation records may be evicted. Only abnormal
> session termination leaves records requiring TTL-based cleanup.

> r[retry.gc.fail-closed]
>
> Expiry MUST fail closed. If an operation record has been evicted and the
> client retries, the server MUST reject the retry with an explicit error —
> it MUST NOT silently treat the evicted ID as Absent and re-execute. Operation
> IDs SHOULD encode enough structure (e.g., a session ID and monotonic sequence)
> that the server can distinguish evicted IDs from genuinely new ones.

> r[retry.gc.deduplicable-durability]
>
> For deduplicable methods with durable effects, operation records SHOULD be
> persisted alongside the effects (same store, same retention policy). Records
> are only safe to evict when the client can no longer plausibly retry — after
> the client has acknowledged receipt of the result, or after the TTL expires.

# Multi-server deployments

> r[retry.distributed]
>
> Deduplicable semantics require that the operation log is accessible to any
> server that might handle a retry.

> r[retry.distributed.rerunnable]
>
> For rerunnable methods, a local operation log is sufficient — re-execution
> on a different server is safe by definition.

> r[retry.distributed.deduplicable]
>
> For deduplicable methods, if the client's retry hits a different server after
> failover and that server has no access to the operation log, it would see the
> operation as Absent and re-execute — violating at-most-once. The operation log
> MUST be shared (replicated store, consensus protocol, or sticky routing).

# Summary of contracts

> r[retry.contract]
>
> The retry model distributes obligations across three parties.

> r[retry.contract.runtime]
>
> **Runtime obligations:** provide operation IDs, maintain the state machine,
> expose the seal/commit API, handle parked duplicates, manage operation log
> lifetime with safe eviction, and surface uncertainty honestly.

> r[retry.contract.handler]
>
> **Handler obligations:** choose the correct category for each method.
> For rerunnable: ensure natural idempotency. For deduplicable: satisfy the
> atomicity obligation (pre-commit failure must not leak effects; commit
> the seal durably). For non-retryable: provide a separate query mechanism
> so clients can resolve ambiguous outcomes.

> r[retry.contract.caller]
>
> **Caller obligations:** mint a unique operation ID per logical operation.
> On ambiguous failure, retry with the same ID. On explicit cancellation,
> use a new operation ID for a new attempt. Distinguish "sealed failure
> replayed" (the operation is done, the answer is an error) from "rejected
> as indeterminate" (the operation's fate is unknown, use other means).
