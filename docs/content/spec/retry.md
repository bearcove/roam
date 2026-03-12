+++
title = "Retry"
description = "Retry safety and semantics in roam"
weight = 13
+++

The retry layer defines how roam handles ambiguous failures — cases where the
client does not know whether the server received, started, or completed a
request. It sits above the transport and session layers and below application
logic.

# The fundamental ambiguity

After any communication failure, the client faces irreducible uncertainty.
The previous attempt is in one of these conditions, and the client cannot
distinguish them:

  1. The request never left the client's outbound buffer
  2. The request arrived but the handler never started
  3. The handler started and is still running
  4. The handler completed but the response was lost in transit
  5. The handler started and then failed, with or without partial side effects

Any retry mechanism must handle all five as possible realities behind a
single "unknown" from the client's perspective. The design that follows
does not pretend the client can tell these apart.

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

The server maintains a record mapping operation IDs to states. A logical
operation proceeds through this lifecycle:

```
              ┌──────────────┐
              │   Absent     │  ← no record exists
              └──────┬───────┘
                     │ first attempt arrives
                     ▼
              ┌──────────────┐
         ┌────│    Live      │────┐
         │    └──────┬───────┘    │
         │           │            │
   abort/cancel      │ handler    │ crash or cancel
   (pre-commit)      │ seals      │ (post-commit,
         │           │ outcome    │  outcome lost)
         ▼           ▼            ▼
  ┌──────────┐  ┌──────────┐  ┌──────────────┐
  │ Released │  │ Sealed   │  │Indeterminate │
  │(→ Absent)│  │(outcome) │  │              │
  └──────────┘  └──────────┘  └──────────────┘
```

> r[retry.state.absent]
>
> **Absent.** No record of this operation ID. A new attempt triggers normal
> handler dispatch and transitions to Live.

> r[retry.state.live]
>
> **Live.** The handler is currently executing. The operation has not yet
> committed an outcome. There MUST be at most one live handler execution for
> any given operation ID.

> r[retry.state.sealed]
>
> **Sealed(outcome).** A terminal outcome — success or failure — has been
> recorded. Retries replay the sealed outcome without re-invoking the handler.

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

When a retry arrives with an operation ID that the server already knows
about, the server's behavior depends on the operation's current state.

> r[retry.duplicate.absent]
>
> If Absent: admit the attempt and start the handler. Transition to Live.

> r[retry.duplicate.live]
>
> If Live: do NOT start a second handler. The duplicate attempt MUST attach
> to the existing in-progress operation and wait for the same outcome.

> r[retry.duplicate.live.broadcast]
>
> When the handler finishes, all attached attempts MUST receive the same
> result.

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

> r[retry.category.rerunnable.contract]
>
> **Handler obligation:** the handler MUST tolerate being run more than once
> for the same logical inputs. No additional cooperation with the runtime is
> required.

> r[retry.category.rerunnable.reexecution]
>
> The runtime MAY re-execute a rerunnable handler on any retry attempt,
> regardless of operation state.

> r[retry.category.rerunnable.caching]
>
> The runtime MAY return a cached result instead of re-executing. Operation
> state tracking is an optimization for rerunnable methods, not a correctness
> requirement.

> r[retry.category.rerunnable.indeterminate]
>
> On Indeterminate state (crash recovery), the runtime MUST re-execute the
> handler.

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

> r[retry.category.deduplicable.contract]
>
> **Handler obligation — atomicity:** if the handler fails or is interrupted
> before it explicitly seals its outcome, any side effects it produced MUST
> be either rolled back or arranged so that a fresh re-execution from the
> beginning will subsume or correct them. Pre-commit failure MUST leave the
> world in a state where re-execution is safe.

> r[retry.category.deduplicable.no-reexecution]
>
> After a deduplicable handler seals its outcome, re-execution MUST NOT
> occur. The runtime MUST replay the cached result for any subsequent
> attempt with the same operation ID.

> r[retry.category.deduplicable.indeterminate]
>
> On Indeterminate state: the runtime MUST inspect durable state. If the
> seal is present, replay it. If the seal is absent, re-execute the handler.
> This is only correct because the atomicity obligation guarantees that
> unsealed effects did not persist.

> r[retry.category.deduplicable.seal-durability]
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

> r[retry.category.non-retryable.contract]
>
> **Handler obligation:** the method SHOULD provide a separate query
> mechanism so callers can resolve ambiguous outcomes (e.g., a read method
> that checks whether the effect happened).

> r[retry.category.non-retryable.indeterminate]
>
> On Indeterminate state, the runtime MUST reject the retry with a clear
> error indicating that the outcome is unknown and cannot be safely retried.
> The client MUST investigate through other means or issue a new logical
> operation with a new operation ID.

# Sealing

Sealing is the point of no return for a logical operation. Once sealed, the
outcome is final — it will be replayed on any future retry, whether the
outcome is success or failure.

> r[retry.seal]
>
> The runtime MUST provide an explicit seal/commit operation in the handler
> API, allowing the handler to mark the point of no return.

> r[retry.seal.implicit]
>
> Returning a success value from a handler implicitly seals the operation.

> r[retry.seal.explicit]
>
> A handler MAY explicitly seal before returning — for example, to record
> that irreversible work has been done and the answer is an error
> ("seal-then-fail"). This MUST be expressible in the handler API.

If the handler committed side effects and then reports failure, that failure
is the true outcome of the operation. Replaying it on retry is correct —
re-executing would attempt to repeat the committed effects. That's why
sealed failures are replayed, not optimistically retried.

> r[retry.seal.pre-commit-release]
>
> If the handler failed before sealing — before any durable side effects —
> the runtime MUST release the operation. This allows the client to retry
> and get a fresh execution.

> r[retry.seal.terminal-replay]
>
> A sealed failure MUST be replayed on retry, not optimistically retried.
> A retry of the same logical operation MUST NOT turn a sealed validation
> error into success.

> r[retry.seal.absorbing]
>
> Once an operation is sealed, no subsequent event — cancellation,
> disconnect, crash recovery — can unseal it. Sealed is absorbing.

# Transient errors

Not all failures come from connection loss. A handler might run successfully
at the protocol level — request delivered, handler invoked, response sent
back — but the handler itself hit a transient downstream failure: a database
timeout, a third-party API returning 503, a lock contention retry. The
connection is fine. The RPC layer didn't fail. But the operation failed in
a way that's worth retrying.

Today, the caller sees an error and has to decide on its own whether to
retry. The handler knows the failure is transient but has no way to say so.
This section gives the handler that voice.

> r[retry.transient.signal]
>
> The handler API MUST provide a way for the handler to mark an error as
> transient. A transient error indicates that the handler performed no
> durable side effects and that re-execution with the same arguments is
> expected to succeed.

> r[retry.transient.release]
>
> A transient error MUST NOT seal the operation. The runtime MUST release
> the operation (transition to Released / Absent), exactly as with any
> pre-commit failure (see `r[retry.seal.pre-commit-release]`).

> r[retry.transient.wire]
>
> The response MUST carry a flag or field indicating that the error is
> transient. This signal is part of the wire format, not just an
> application-level convention.

> r[retry.transient.caller-policy]
>
> The caller's retry policy decides whether and how to act on a transient
> error signal. The runtime MUST expose the transient flag to the caller.
> The caller MAY implement backoff, jitter, and maximum attempt limits.

> r[retry.transient.retry-after]
>
> The response MAY carry a retry-after hint (a duration) alongside the
> transient flag. The caller SHOULD respect this hint when present.

# Cancellation interaction

Cancellation interacts with the operation state machine. There are two
distinct events that look like cancellation: the client actively requesting
abort, and the client simply disappearing.

> r[retry.cancel.explicit.pre-commit]
>
> If the client explicitly cancels and the operation has not yet sealed,
> the handler SHOULD be aborted and the operation released.

> r[retry.cancel.explicit.post-commit]
>
> If the client explicitly cancels but the operation is already sealed,
> cancellation is a no-op. The sealed outcome stands.

> r[retry.cancel.implicit.rerunnable]
>
> When the client disconnects during a rerunnable method, the server MAY
> abort the handler to save resources. Re-execution on reconnect is safe.

> r[retry.cancel.implicit.deduplicable]
>
> When the client disconnects during a deduplicable method, the server
> SHOULD continue the handler to completion and cache the outcome, so that
> when the client retries, the answer is waiting.

> r[retry.cancel.implicit.non-retryable]
>
> When the client disconnects during a non-retryable method, the server
> MAY continue or abort. Either way, the operation state MUST accurately
> reflect what happened.

> r[retry.cancel.race]
>
> Cancellation competes with commit. Whichever transition (cancel-and-release
> vs. seal) reaches the operation record first wins. The client MUST be
> prepared for either outcome.

> r[retry.cancel.retry-after]
>
> A retry with the same operation ID after a cancel request MUST reattach to
> the same operation, not create a new one. If the operation sealed as
> cancelled, retries replay the cancelled outcome. To try again from scratch,
> the client MUST use a new operation ID.

# Attempt failure vs. operation outcome

These are distinct concepts, and conflating them is a common source of bugs.

**Attempt failures** are failures of a particular delivery/execution attempt:
connection dropped, timeout waiting for response, process died before durable
seal. These are NOT automatically operation outcomes. The operation may still
be Live on the server, or it may have sealed successfully with the response
lost in transit.

**Operation outcomes** are the final outcomes of the logical operation:
success, business rejection, terminal failure. Transparent retry is defined
over operations, not over attempts. A transient pre-commit crash does not
seal the operation. A validation error does. A post-commit failure does.

# Reconnection model

The session is the thing with identity and state. The conduit is just the
pipe. When the pipe breaks, you get a new pipe and continue the same
session.

Recovery is a two-step process: first try conduit-level reconnection, and
if that fails, resume the session on a new conduit.

## Conduit-level reconnection

A `StableConduit` (see `r[conduit.stable]`) handles link failures
transparently — it reconnects over a fresh link and replays missed
messages. The session doesn't even notice the interruption. This is the
cheapest recovery path and should be tried first.

> r[retry.reconnect.stable-conduit]
>
> When a `StableConduit` successfully reconnects and replays missed
> messages, the session MUST continue as if the link never failed. No
> operation-level retry is triggered.

## Session resumption

If conduit-level reconnection fails — `BareConduit` link failure, or a
`StableConduit` that could not recover — the session resumes on a new
conduit. The conduit is dead, but the session is not. The client obtains
a new conduit and presents the existing session's identity. All session
state — operation records, in-flight requests, connection state — is
preserved because it's the same session, just on a new pipe.

This is the primary scenario the retry machinery is designed for. The
operation ID scope is the session (see `r[retry.op-id.scope]`), so as
long as the session survives, retry works.

> r[retry.reconnect.session-resume]
>
> A session MUST be resumable on a new conduit. When the underlying conduit
> fails, the session MUST NOT be torn down. The server MUST retain session
> state (operation records, connection state, channel state) until the
> client resumes or the session is explicitly closed.

> r[retry.reconnect.session-resume.handshake]
>
> Session resumption MUST use a resume handshake that presents the existing
> session's identity to the server. The server MUST validate the session
> identity and, if the session is still alive, continue it on the new
> conduit.

> r[retry.reconnect.session-resume.ambiguous-ops]
>
> After session resumption, the session layer MUST determine which
> in-flight operations have ambiguous delivery status. For each ambiguous
> operation, the operation layer re-sends the request as an explicit retry
> attempt with the original operation ID.

> r[retry.reconnect.session-resume.reattach]
>
> If the server confirms that an in-flight operation is still Live after
> session resumption, the client MAY wait for the result over the resumed
> session without re-sending the request.

> r[retry.reconnect.session-resume.channels]
>
> Channels that were active before the conduit failure are terminated by
> the connection loss (see `r[retry.channel.connection-bound]`). After
> session resumption, rerunnable methods with channels are re-executed and
> channel handles are rebound per `r[retry.channel.rebinding]`.

If session resumption fails — the server has no record of the session
because it crashed and lost state — then the client is starting from
scratch. New session, new identity, new operation IDs. There is no retry
of old operations in this case; the server is gone and has no memory of
what came before.

## Transport layer obligations

> r[retry.layers.transport-retransmit]
>
> If the transport knows a message was never transmitted (still in the send
> buffer when the connection dropped), it MAY retransmit transparently —
> this is below the operation layer's concern.

> r[retry.layers.no-silent-retry]
>
> If the transport does NOT know whether a message reached the server, it
> MUST surface this uncertainty to the operation layer. The transport MUST
> NOT silently retry operations.

# Operation record lifetime

The server cannot keep operation records forever, but premature eviction
is dangerous: if a deduplicable operation's record is evicted and the client
retries, the server would re-execute (seeing the ID as Absent), violating
at-most-once.

> r[retry.gc.ttl]
>
> Operation records MUST have a TTL that exceeds the maximum retry window
> by a comfortable margin.

> r[retry.gc.ttl.start]
>
> TTL countdown MUST start only after the operation reaches a terminal state,
> not from request arrival.

> r[retry.gc.live-protected]
>
> Live operations MUST NOT be evicted while the handler is alive.

> r[retry.gc.session-scoped]
>
> When a session ends cleanly, all its operation records MAY be evicted.
> Only abnormal session termination leaves records requiring TTL-based
> cleanup.

> r[retry.gc.fail-closed]
>
> Expiry MUST fail closed. If an operation record has been evicted and the
> client retries, the server MUST reject the retry with an explicit error —
> it MUST NOT silently treat the evicted ID as Absent and re-execute.

> r[retry.gc.id-structure]
>
> Operation IDs SHOULD encode enough structure (e.g., a session ID and
> monotonic sequence) that the server can distinguish evicted IDs from
> genuinely new ones.

> r[retry.gc.deduplicable-persistence]
>
> For deduplicable methods with durable effects, operation records SHOULD be
> persisted alongside the effects (same store, same retention policy). Records
> are only safe to evict when the client can no longer plausibly retry — after
> the client has acknowledged receipt of the result, or after the TTL expires.

# Channels and retry

Channels (see `r[rpc.channel]`) are connection-bound, stateful streams. They
don't naturally compose with retry the way a stateless request/response pair
does. This section defines how channels interact with the retry machinery.

The motivating pattern is "seed + deltas": a method like
`watch_room(room_id, events: Tx<RoomEvent, 16>)` where the handler first
sends a full state dump (the seed), then streams incremental updates. On
reconnect, the method is re-executed and the new handler sends a fresh seed.
The client's `Rx` handle is transparently rebound to the new channel — it
just sees a new Seed arrive and resets its local state. No special handling,
no awareness that a retry occurred.

This works because the handler always starts with a seed. The seed IS the
synchronization point. No acknowledgment or replay machinery is needed —
re-execution produces a fresh, self-contained stream.

Transparent rebinding does NOT work for channels where the client is
sending items to the server (command channels, mutation streams). After
reconnection, a new handler starts from scratch with no knowledge of what
the old handler received. The client has no way to know which items were
consumed. Reliable bidirectional streaming that survives reconnection is
a different abstraction (durable subscriptions, topic-based messaging) and
is out of scope for the retry layer.

> r[retry.channel.connection-bound]
>
> Channels are bound to the connection they were created on. When a
> connection is lost, all channels on that connection are terminated.

> r[retry.channel.no-sealed-replay]
>
> Sealed replay MUST NOT attempt to re-establish channels. A sealed outcome
> contains the return value, not live channel state. Methods whose usefulness
> depends entirely on their channels (e.g., a streaming method that returns
> `()`) gain nothing from sealed replay — the caller must issue a new
> operation.

> r[retry.channel.rerunnable-reexecution]
>
> When a rerunnable method with channels is re-executed on retry, the
> runtime MUST create fresh channels for the new execution. The handler
> receives new channel handles and starts from scratch.

> r[retry.channel.rebinding]
>
> When a rerunnable method is re-executed on retry, the caller's original
> channel handles (the paired ends it kept) MUST be transparently rebound
> to the fresh channels from the new execution. The caller MUST NOT need
> to create new channel pairs or be aware that a retry occurred.

> r[retry.channel.rebinding.rx]
>
> An `Rx<T>` handle whose underlying channel was terminated by connection
> loss MUST, on the next `recv()` call, receive items from the replacement
> channel created by re-execution. Items already consumed from the original
> channel are not replayed — the new channel starts fresh (which is safe
> because the method is rerunnable and the handler will re-seed).

> r[retry.channel.rebinding.tx]
>
> A `Tx<T>` handle whose underlying channel was terminated by connection
> loss MUST, on the next `send()` call, send items through the replacement
> channel created by re-execution.

> r[retry.channel.deduplicable-no-rebinding]
>
> For deduplicable methods, channel rebinding MUST NOT occur. If the
> operation is Live and a duplicate joins, the duplicate waits for the
> return value — it does not get access to the running handler's channels.
> If the operation is Sealed, the caller receives the cached return value
> with no live channels.

# Summary

The retry model distributes obligations across three parties:

**The runtime** provides operation IDs, maintains the state machine, exposes
the seal/commit API, handles parked duplicates, manages operation log
lifetime with safe eviction, and surfaces uncertainty honestly.

**The handler** chooses the correct category for each method. For rerunnable:
ensure natural idempotency. For deduplicable: satisfy the atomicity
obligation. For non-retryable: provide a separate query mechanism.

**The caller** mints a unique operation ID per logical operation, retries
with the same ID on ambiguous failure, and uses a new ID when starting a
genuinely new operation. The caller must distinguish "sealed failure replayed"
(the operation is done, the answer is an error) from "rejected as
indeterminate" (the operation's fate is unknown).
