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
   abort/cancel      │ handler    │ crash (between
   (pre-commit)      │ calls      │ commit and seal,
         │           │ seal()     │ outcome lost)
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
> Live, between commit and seal. The effect may have happened but the
> outcome was never recorded. For idempotent methods, the runtime re-executes.
> For effectful methods, recovery depends on whether durable state can be
> inspected (see `r[retry.category.effectful.indeterminate]`).

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
> Every service method MUST be assigned one of two retry categories. The
> category determines what guarantees the runtime can provide and what
> obligations the handler must satisfy.

## Idempotent

The simplest category. An idempotent method is one where running it again is
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
for idempotent methods is performance, not correctness.

> r[retry.category.idempotent.contract]
>
> **Handler obligation:** the handler MUST tolerate being run more than once
> for the same logical inputs. No additional cooperation with the runtime is
> required.

> r[retry.category.idempotent.reexecution]
>
> The runtime MAY re-execute an idempotent handler on any retry attempt,
> regardless of operation state.

> r[retry.category.idempotent.caching]
>
> The runtime MAY return a cached result instead of re-executing. Operation
> state tracking is an optimization for idempotent methods, not a correctness
> requirement.

> r[retry.category.idempotent.indeterminate]
>
> On Indeterminate state (crash recovery), the runtime MUST re-execute the
> handler.

## Effectful

An effectful method is NOT safe to run twice on its own — but the runtime
and handler cooperate to make retry safe through two explicit lifecycle
points: **commit** and **seal**.

The handler controls both points:

- **Commit** marks the point of no return. Before commit, the handler has
  not performed any irreversible effects — re-execution is safe. After
  commit, effects may have escaped (a database write, an API call, a
  message sent).

- **Seal** records the final outcome. After seal, retries receive the
  cached result — the handler is never re-invoked.

The critical rule: **commit before the dangerous `.await`**. If the handler
is about to perform an irreversible operation — an external API call, a
database write that can't be rolled back — it must call `commit()` first.
If the process crashes during the `.await`, the runtime knows the operation
is past the point of no return and will not blindly re-execute.

```rust
// CORRECT: commit before the irreversible await
ctx.commit();
let response = http_post(url, payload).await;
ctx.seal(response);

// WRONG: the webhook fires, then we crash before commit —
// runtime thinks re-execution is safe, webhook fires again
let response = http_post(url, payload).await;
ctx.commit(); // too late, the effect already escaped
```

The gap between commit and seal is the **danger window**. If the process
crashes in this window, the operation is Indeterminate — the effect may
have happened, but there's no cached outcome to replay.

How wide this window is depends on the handler's design:

**Narrow window (database transactions):** For methods like
`transfer_money(from, to, amount)`, the handler can write the balance
changes and seal the operation ID in the same database transaction. Commit
and seal happen atomically. If the transaction commits, the seal is there —
retries get replays. If it doesn't, nothing happened — re-execution is
safe. The Indeterminate window is essentially zero.

**Wide window (external effects):** For methods like
`send_webhook(url, payload)`, the handler POSTs to a third party. Commit
happens before the POST; seal happens after the 200 comes back. If the
process crashes between the POST and the seal, the webhook fired but
there's no cached outcome. The operation is Indeterminate, and the runtime
must be honest about it.

**Examples across the spectrum:**

- `transfer_money(from, to, amount)` — commit + seal in one DB transaction.
  Zero-width danger window. Exactly-once execution guaranteed.

- `create_order(customer_id, items)` — same pattern: order creation + seal
  in one transaction. Retries replay the same order ID.

- `send_webhook(url, payload)` — commit, then POST, then seal. Wide danger
  window. If the process crashes mid-POST, the outcome is unknown.

- `send_sms(phone_number, message)` — commit, then send via carrier, then
  seal. The SMS is gone the moment the carrier accepts it. If we crash
  before sealing, the operation is Indeterminate.

- `trigger_deploy(service, version)` — commit, then kick off CI pipeline,
  then seal. Once the pipeline starts, there's no undo.

For methods with a wide danger window, the handler SHOULD provide a
separate query mechanism (like `get_webhook_delivery_status`) so callers
can resolve ambiguous outcomes.

> r[retry.category.effectful.commit]
>
> The runtime MUST provide an explicit `commit()` operation in the handler
> API. Calling `commit()` marks the point of no return — the handler is
> about to perform irreversible effects.

> r[retry.category.effectful.commit-before-await]
>
> The handler MUST call `commit()` before any `.await` (or other yield
> point) that performs an irreversible operation. If the handler crashes
> during an irreversible `.await` without having committed, the runtime
> will assume re-execution is safe — potentially causing duplicate effects.

> r[retry.category.effectful.pre-commit-safe]
>
> Before `commit()` is called, the operation is safe to abort and re-execute.
> If the handler fails or is interrupted before committing, the runtime
> MUST release the operation (transition to Released / Absent), allowing
> a fresh execution on retry.

> r[retry.category.effectful.seal]
>
> The runtime MUST provide an explicit `seal(outcome)` operation in the
> handler API. Sealing records the final outcome (success or failure) so
> that retries receive the cached result without re-invoking the handler.

> r[retry.category.effectful.seal-after-commit]
>
> `seal()` MUST be called after `commit()`. The handler commits (point of
> no return), performs the effect, observes the result, and then seals the
> outcome.

> r[retry.category.effectful.implicit-seal]
>
> Returning a success value from an effectful handler implicitly seals the
> operation with that value. Explicit `seal()` is needed only when the
> handler wants to record an outcome before returning — for example, to
> seal a success and then return an error ("seal-then-fail").

> r[retry.category.effectful.no-reexecution]
>
> After an effectful handler seals its outcome, re-execution MUST NOT
> occur. The runtime MUST replay the cached result for any subsequent
> attempt with the same operation ID.

> r[retry.category.effectful.indeterminate]
>
> On Indeterminate state (crash between commit and seal): the runtime MUST
> inspect durable state. If the seal is present, replay it. If the seal is
> absent but the operation was committed, the outcome is genuinely unknown.
> The runtime MUST report this to the client as an indeterminate error.
> If the operation was never committed, re-execution is safe.

> r[retry.category.effectful.seal-durability]
>
> For effectful methods, the seal MUST be durable. An in-memory-only seal
> is not a real seal — a crash would lose it and violate at-most-once.

> r[retry.category.effectful.narrow-window]
>
> When possible, the handler SHOULD arrange for commit and seal to happen
> atomically (e.g., in the same database transaction). This eliminates the
> Indeterminate window entirely.

> r[retry.category.effectful.query-mechanism]
>
> For effectful methods with a wide commit-to-seal window (external API
> calls, messages to third parties), the handler SHOULD provide a separate
> query mechanism so callers can resolve ambiguous outcomes.

# Sealing properties

The commit/seal lifecycle is defined in the effectful method category (see
`r[retry.category.effectful.commit]` through
`r[retry.category.effectful.seal-durability]`). This section covers
properties of sealed outcomes that apply regardless of method category.

If the handler committed side effects and then reports failure, that failure
is the true outcome of the operation. Replaying it on retry is correct —
re-executing would attempt to repeat the committed effects. That's why
sealed failures are replayed, not optimistically retried.

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
> If the client explicitly cancels and the operation has not yet committed,
> the handler SHOULD be aborted and the operation released.

> r[retry.cancel.explicit.post-seal]
>
> If the client explicitly cancels but the operation is already sealed,
> cancellation is a no-op. The sealed outcome stands.

> r[retry.cancel.explicit.committed-unsealed]
>
> If the client explicitly cancels and the operation has committed but not
> yet sealed, the server SHOULD continue the handler to completion and seal
> the outcome. Aborting a committed operation leaves it Indeterminate.

> r[retry.cancel.implicit.idempotent]
>
> When the client disconnects during an idempotent method, the server MAY
> abort the handler to save resources. Re-execution on reconnect is safe.

> r[retry.cancel.implicit.effectful.pre-commit]
>
> When the client disconnects during an effectful method that has not yet
> committed, the server MAY abort the handler and release the operation.
> Re-execution on retry is safe because no irreversible effects have occurred.

> r[retry.cancel.implicit.effectful.post-commit]
>
> When the client disconnects during an effectful method that has already
> committed, the server SHOULD continue the handler to completion and seal
> the outcome. Aborting a committed operation leaves it Indeterminate.

> r[retry.cancel.race]
>
> Cancellation competes with commit. If cancellation reaches the operation
> before the handler calls `commit()`, the operation is released. If
> `commit()` wins, the operation proceeds to completion. The client MUST
> be prepared for either outcome.

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

This path is particularly attractive for zero-copy transports (e.g.,
shared memory) where `StableConduit` buffering overhead is unacceptable.
A `BareConduit` pays nothing in the happy path — no replay log, no
message buffering. On failure, the caller still owns the original
arguments (they were borrowed for serialization, not consumed), so
retrying is just: same operation ID, re-serialize from the same source.
No pre-emptive copies needed.

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

> r[retry.reconnect.session-resume.reserialize]
>
> When retrying an ambiguous operation after session resumption, the runtime
> MUST re-serialize the arguments from the caller's original data. The
> runtime MUST NOT require pre-emptive copies of serialized request payloads
> for retry purposes.

> r[retry.reconnect.session-resume.reattach]
>
> If the server confirms that an in-flight operation is still Live after
> session resumption, the client MAY wait for the result over the resumed
> session without re-sending the request.

> r[retry.reconnect.session-resume.channels]
>
> Channels that were active before the conduit failure are terminated by
> the connection loss (see `r[retry.channel.connection-bound]`). After
> session resumption, idempotent methods with channels are re-executed and
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
is dangerous: if an effectful operation's record is evicted and the client
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

> r[retry.gc.effectful-persistence]
>
> For effectful methods with durable effects, operation records SHOULD be
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

> r[retry.channel.idempotent-reexecution]
>
> When an idempotent method with channels is re-executed on retry, the
> runtime MUST create fresh channels for the new execution. The handler
> receives new channel handles and starts from scratch.

> r[retry.channel.rebinding]
>
> When an idempotent method is re-executed on retry, the caller's original
> channel handles (the paired ends it kept) MUST be transparently rebound
> to the fresh channels from the new execution. The caller MUST NOT need
> to create new channel pairs or be aware that a retry occurred.

> r[retry.channel.rebinding.rx]
>
> An `Rx<T>` handle whose underlying channel was terminated by connection
> loss MUST, on the next `recv()` call, receive items from the replacement
> channel created by re-execution. Items already consumed from the original
> channel are not replayed — the new channel starts fresh (which is safe
> because the method is idempotent and the handler will re-seed).

> r[retry.channel.rebinding.tx]
>
> A `Tx<T>` handle whose underlying channel was terminated by connection
> loss MUST, on the next `send()` call, send items through the replacement
> channel created by re-execution.

> r[retry.channel.effectful-no-rebinding]
>
> For effectful methods, channel rebinding MUST NOT occur. If the
> operation is Live and a duplicate joins, the duplicate waits for the
> return value — it does not get access to the running handler's channels.
> If the operation is Sealed, the caller receives the cached return value
> with no live channels.

# Summary

The retry model distributes obligations across three parties:

**The runtime** provides operation IDs, maintains the state machine, exposes
the commit/seal API, handles parked duplicates, manages operation log
lifetime with safe eviction, and surfaces uncertainty honestly.

**The handler** chooses the correct category for each method. For idempotent:
ensure natural idempotency. For effectful: call `commit()` before
irreversible effects and `seal()` after observing the outcome. When commit
and seal can be atomic (e.g., same database transaction), the Indeterminate
window is eliminated entirely.

**The caller** mints a unique operation ID per logical operation, retries
with the same ID on ambiguous failure, and uses a new ID when starting a
genuinely new operation. The caller must distinguish "sealed failure replayed"
(the operation is done, the answer is an error) from "rejected as
indeterminate" (the operation's fate is unknown).
