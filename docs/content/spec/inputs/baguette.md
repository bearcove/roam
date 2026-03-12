# Retry Semantics for RPC: A First-Principles Design

## The fundamental ambiguity

After any communication failure, the client faces irreducible uncertainty about server-side state. The previous attempt is in one of five conditions, and the client cannot distinguish them:

1. The request never left the client's outbound buffer.
2. The request arrived but the handler never started.
3. The handler started and is still running.
4. The handler completed but the response was lost in transit.
5. The handler started and then failed, with or without partial side effects.

Any retry model that pretends the client can distinguish these is wrong from the start. The correct design must handle all five as possible realities behind a single "I don't know" from the client.

---

## Operation identity is necessary

Without an explicit identifier tying retry attempts to the same logical operation, the server cannot distinguish "the client is retrying attempt #1 of operation X" from "the client is issuing a new operation Y that happens to have identical arguments." This distinction is essential for any method where running the handler twice would produce different observable effects than running it once.

Even for naturally idempotent methods, operation identity is useful as an optimization (skip redundant work if the result is already cached). But for non-idempotent methods, it is load-bearing: without it, you cannot offer at-most-once execution.

**Definition.** A *logical operation* is the client's intention to cause exactly one execution of a particular method with particular arguments, yielding exactly one outcome. The client mints a unique operation ID for each logical operation. Every delivery attempt for that operation carries the same ID. A new intention — even with identical arguments — gets a new ID.

This places the identity boundary at the right level. The server never has to guess whether two requests are "the same" based on argument equality or timing heuristics.

---

## The operation state machine

The server maintains a log (durable or in-memory depending on the reliability tier) mapping operation IDs to states. A logical operation proceeds through this lifecycle:

```
                ┌──────────────┐
                │  Unrecognized │  ← no record exists
                └──────┬───────┘
                       │ first attempt arrives
                       ▼
                ┌──────────────┐
           ┌────│   Running     │────┐
           │    └──────┬───────┘    │
           │           │            │
     abort/cancel      │ handler    │ crash or cancel
     (pre-commit)      │ seals      │ (post-commit,
           │           │ outcome    │  outcome lost)
           ▼           ▼            ▼
    ┌──────────┐  ┌──────────┐  ┌──────────────┐
    │Released  │  │ Sealed   │  │  Indeterminate│
    │(→ Unrec.)│  │(outcome) │  │  (see below)  │
    └──────────┘  └──────────┘  └──────────────┘
```

**Unrecognized.** No record of this operation ID. A new attempt triggers normal handler dispatch and transitions to Running.

**Running.** The handler is currently executing. The operation has not yet committed an outcome.

**Sealed(outcome).** A terminal outcome — `Success(value)` or `Failure(error)` — has been durably recorded. This is the absorbing state under normal operation.

**Released.** The operation was aborted before committing, and the server has confirmed no side effects leaked. The operation ID effectively returns to Unrecognized. A subsequent attempt gets a fresh execution.

**Indeterminate.** The server crashed or lost state while the operation was Running. The outcome is genuinely unknown. This state exists to be honest about the gap; how it resolves depends on the method category (see below).

---

## Method categories: the minimum necessary set

Different methods have different relationships to re-execution. I think exactly three categories are necessary and sufficient. Fewer collapses important distinctions; more introduces accidental complexity.

### 1. Rerunnable

The handler is naturally idempotent: executing it N times with the same arguments produces the same observable effects as executing it once. Reads, upserts-by-key, pure computations, and "set X to V" operations fall here.

**Contract for the service implementer:** The handler must tolerate being run more than once for the same logical inputs. No additional cooperation with the runtime is required.

**System behavior on duplicate:** The system *may* re-execute the handler (always safe) or return a cached result if one exists (an optimization). The system does not need to maintain operation state for correctness, only for efficiency.

**On indeterminate state (crash recovery):** Re-execute. Safe by definition.

### 2. Deduplicable

The handler is NOT safe to run twice, but the system can enforce at-most-once execution by tracking the operation ID and caching the outcome. A "deduplicable" method can be made exactly-once from the client's perspective if the runtime provides the deduplication machinery.

**Contract for the service implementer:** The handler must satisfy an *atomicity obligation*: if the handler fails or is interrupted before it explicitly commits its outcome (seals), any side effects it produced must be either (a) rolled back, or (b) arranged so that a fresh re-execution from the beginning will subsume or correct them. In other words, pre-commit failure must leave the world in a state where re-execution is safe.

After the handler seals its outcome, re-execution will never occur — the runtime replays the cached result.

This is the strongest and most interesting category. The key insight is that the "idempotency" is not a property of the handler in isolation but of the *pair* (handler + runtime deduplication). The handler only needs to be idempotent in the pre-commit window, which is a much weaker requirement than natural idempotency everywhere.

**System behavior on duplicate:**
- If Running: park the duplicate, wait for the outcome, return it.
- If Sealed: return the cached outcome.
- If Unrecognized: execute normally.

**On indeterminate state:** This is where the atomicity obligation is load-bearing. If the handler's effects and the operation record are committed together (e.g., same transaction, same write-ahead log entry), then on recovery the system can inspect durable state: either the seal is present (replay it) or it isn't (effects didn't persist, re-execute safely). If the handler *cannot* make this guarantee, it does not belong in this category.

### 3. Unreliable

The handler cannot support transparent retry at all. Its effects are not naturally idempotent, and it cannot satisfy the atomicity obligation required for deduplication — perhaps because it touches external systems without their own idempotency support, or because partial effects are irrecoverable.

**Contract for the service implementer:** Acknowledge that the system cannot guarantee exactly-once execution. The method should be designed so that callers can query for the outcome through a separate channel if needed (e.g., a read method that checks whether the effect happened).

**System behavior on duplicate:**
- If Sealed: return the cached outcome. (This is still safe and useful — the response-loss case is handled.)
- If Running: park and wait, same as deduplicable.
- If Unrecognized: execute normally. (The client has accepted the risk.)
- If Indeterminate: *reject* the duplicate with a clear error indicating that the outcome is unknown and cannot be safely retried under this operation ID. The client must investigate through other means or issue a new logical operation.

The difference from deduplicable is entirely about the indeterminate case. An unreliable method is honest that re-execution after ambiguous failure is not safe, rather than pretending it can be papered over.

### Why not fewer categories?

Collapsing rerunnable and deduplicable loses the distinction between "the handler is inherently safe to re-execute" and "the handler needs system help to avoid double execution." This matters for performance (rerunnable methods don't need the operation log on the hot path) and for the crash-recovery story (rerunnable methods recover trivially).

Collapsing deduplicable and unreliable loses the most important distinction of all: whether the system can offer exactly-once semantics after ambiguous failure. The whole point of deduplicable is that the handler has accepted a contract that makes this safe.

### Why not more categories?

You might think "safe to rerun if failed but not if succeeded" deserves its own category. It doesn't — that's just deduplicable. The sealed outcome (whether success or failure) is replayed; before sealing, re-execution is safe by the atomicity obligation.

You might also think read-only methods deserve special treatment. They don't — they're a subset of rerunnable with no new semantic considerations.

---

## The sealing question: should terminal failures be replayed?

This is the most consequential design decision.

**Post-commit failures must be sealed.** If the handler committed side effects and then reported failure (e.g., "wrote A and B but failed to write C"), that failure is the true outcome of the operation. Replaying it on retry is correct — the alternative (re-executing) would attempt to write A and B again, violating at-most-once.

**Pre-commit failures should release the operation ID.** If the handler failed before committing — before any durable side effects — the operation is as if it never ran. Releasing the ID allows the client to retry and get a fresh execution. This is the common case for transient errors (downstream timeout, temporary resource exhaustion), and sealing these would be needlessly punitive.

The dividing line is the handler's own commit point. The runtime provides a mechanism for the handler to say "I am now past the point of no return." Before that signal, failure releases; after it, failure seals.

This means the runtime needs an explicit `seal` or `commit` operation in its handler API, not just the natural return of a result. Returning a success value implicitly seals. But the handler might also need to seal-then-fail: "I've done irreversible work and the answer is an error." This must be expressible.

**Important subtlety:** if the handler never calls seal and the process crashes, the runtime does not know whether effects leaked. For deduplicable methods, the atomicity obligation says they didn't (or that re-execution will fix them). For unreliable methods, the runtime correctly reports indeterminate.

---

## Cancellation and its interaction with retry

There are two distinct events that look like cancellation:

**Explicit cancellation.** The client actively requests that the operation be aborted. Semantics: if the operation is pre-commit, abort the handler and release the operation ID. If post-commit, cancellation is a no-op — the outcome is sealed and the cancel arrives too late. The client should receive a response indicating which case occurred.

**Implicit cancellation (lost interest).** The client disconnected or stopped waiting. The client *might come back*. This is operationally different from explicit cancellation:

- For **rerunnable** methods: the server can abort the handler (saves resources), because re-execution on reconnect is safe.
- For **deduplicable** methods: the server should ideally *continue the handler to completion* and cache the outcome, so that when the client retries, the answer is waiting. Aborting is only safe if the handler is pre-commit and the abort can guarantee no leaked effects.
- For **unreliable** methods: the server's choice. Continuing is polite; aborting is defensible. Either way, the operation state must accurately reflect what happened.

**Cancellation races.** The client sends cancel concurrent with handler completion. The server resolves this by internal ordering: whichever transition (cancel-and-release vs. seal) reaches the operation record first wins. The client must be prepared for either outcome. This is unavoidable in any async system — the design should not try to eliminate the race but should ensure both resolutions leave the system in a consistent state.

**Key invariant:** after an operation is sealed, no subsequent cancellation can unseal it. Sealed is absorbing.

---

## Transport layer vs. operation layer

The transport layer's job is to deliver bytes between client and server, manage connections, handle reconnection, and provide flow control. It should provide at-most-once or at-least-once *message delivery*, but it should NOT silently retry *operations*.

**The critical boundary:** if the transport knows a message was never transmitted (still in the send buffer when the connection dropped), it may retransmit transparently — this is below the operation layer's concern. But if the transport does not know whether the message reached the server, it must surface this uncertainty to the operation layer rather than silently resending.

The operation layer then decides what to do based on the operation ID machinery described above.

In a bidirectional RPC system with connection multiplexing, this means: after a reconnect, the session layer should report which in-flight operations have ambiguous delivery status. The operation layer then re-sends those as explicit retry attempts with the original operation IDs. This is the right decomposition because the transport cannot know whether the handler ran — only the operation layer can determine that via server-side state.

**What about session-layer "transparent reconnection"?** If the session layer can resume a connection and the server confirms that a particular in-flight operation is still Running (via the operation state machine), the session layer can simply wait for the result over the new connection without re-sending the request. This is a valuable optimization but it requires the operation layer's cooperation — specifically, the ability to re-attach a client-side waiter to a server-side in-progress operation.

---

## Operation identity lifetime and garbage collection

The server cannot keep operation records forever. But premature eviction is dangerous: if a deduplicable operation's record is evicted and the client retries, the server will re-execute (seeing the ID as unrecognized), violating at-most-once.

**Principles for safe eviction:**

1. Operation records should have a TTL that exceeds the maximum retry window by a comfortable margin. If clients are configured to retry for up to 30 seconds, a TTL of 10 minutes provides safety margin.

2. Operation IDs should encode enough structure (e.g., a timestamp component, session ID, or monotonic sequence) that the server can distinguish "this is an old ID whose record was evicted" from "this is a new ID I've never seen." Evicted-but-recognized IDs should be rejected, not treated as unrecognized. This prevents silent duplicate execution after record eviction.

3. For deduplicable methods with durable effects, operation records should be persisted alongside the effects (same store, same retention policy). Records are only safe to evict when the client can no longer plausibly retry — which in practice means after the client has acknowledged receipt of the result, or after a generous timeout.

4. Session-scoped operation IDs (IDs meaningful only within a particular client session) simplify lifetime management: when a session ends cleanly, all its operation records can be evicted. Only abnormal session termination leaves records requiring TTL-based cleanup.

---

## Edge cases and hidden failure modes

**Duplicate execution after response loss.** Server seals `Success(v)`, sends response, response is lost. Client retries with same operation ID. Server finds `Sealed(Success(v))`, returns `v`. This is the bread-and-butter case and it works cleanly under this model. The only risk is record eviction before retry; the encoding-based rejection described above handles that.

**In-progress duplicate arrival.** Client retries while first attempt is still Running. The duplicate "joins" the in-progress operation — the server parks the second request and delivers the same outcome to both when the handler finishes. This must handle the sub-case where the handler fails: both the original and the duplicate receive the failure (and the operation is released or sealed depending on commit state).

**The park-then-cancel race.** Client A sends attempt 1. Client A disconnects. Client A reconnects and sends attempt 2 (same operation ID). Meanwhile, the server is aborting attempt 1 due to lost-interest cancellation. If the abort completes and releases the operation ID *before* attempt 2 arrives, attempt 2 triggers fresh execution — correct. If attempt 2 arrives *before* the abort completes, it parks on the Running state. Then the abort transitions to Released, and... the parked duplicate must be woken up and told "the operation was released; you may re-execute." This requires careful coordination. The simplest correct implementation: when an operation transitions to Released, all parked duplicates are woken with a "retry now" signal rather than being given a result.

**Seal-then-crash.** The handler calls seal with `Success(v)`, the seal is written to durable storage, and then the process crashes before the response reaches the client. On recovery, the server restores `Sealed(Success(v))` from the log. Client retries; gets `v`. Correct. The risk: if the seal was in memory but not yet durable, the operation appears as Indeterminate on recovery. This is why the seal operation must be durable for deduplicable methods — an in-memory-only seal is not a real seal.

**Partial effects with transactional stores.** If the handler writes to a transactional store and commits both its effects and the seal in the same transaction, crash recovery is clean: either the transaction committed (sealed) or it didn't (safe to re-execute). This is the gold path for deduplicable methods. When the handler's effects span multiple non-transactional systems, the atomicity obligation becomes much harder to satisfy — the handler may need to implement saga-style compensation, or the method should honestly be classified as unreliable.

**Ambiguous failure after partial effects in unreliable methods.** Handler writes to an external system (side effect leaks), then crashes. On retry, the server sees Indeterminate and rejects. Now the client knows the outcome is ambiguous and must query the external system to determine what happened. This is the correct behavior — the system is honest rather than optimistic.

**Clock skew and operation ID timestamps.** If operation IDs contain timestamps for generation-based rejection, clock skew between client and server can cause valid IDs to be rejected as "too old." Mitigation: use logical clocks (session-scoped sequence numbers) rather than wall-clock timestamps when possible.

**Long-running handlers and TTL.** If a handler runs for longer than the operation record TTL... this shouldn't happen if the TTL is configured correctly (it should be measured from *completion*, not from request arrival). But the system should guarantee that Running operations are never evicted while the handler is alive. TTL countdown should start only after the operation reaches a terminal state.

**Multi-server deployments.** If the operation log is local to a single server and the client's retry hits a different server after failover, the new server sees the operation as Unrecognized. For rerunnable methods: fine. For deduplicable methods: violation of at-most-once unless the operation log is shared (replicated store, consensus protocol, or sticky routing). This is an operational concern that the semantic model should make explicit: **deduplicable semantics require that the operation log is accessible to any server that might handle a retry.** The model should not pretend single-server logs are sufficient when they aren't.

---

## Summary of the contract

**For the RPC system implementer:** provide operation IDs, the state machine described above, per-method category declarations, the seal/commit API, parked-duplicate handling, and operation log management with safe eviction.

**For the service implementer:** choose the correct category for each method. For rerunnable methods: ensure natural idempotency. For deduplicable methods: satisfy the atomicity obligation (pre-commit failure must not leak effects that would make re-execution unsafe; commit the seal durably). For unreliable methods: provide a separate query mechanism so clients can resolve ambiguous outcomes.

**For the client:** mint a unique operation ID per logical operation. On ambiguous failure, retry with the same ID. On explicit cancellation, issue a cancel and then (if retrying the intent) use a new operation ID for the new attempt. Respect the distinction between "sealed failure replayed to me" (the operation is done, the answer is an error) and "rejected as indeterminate" (the operation's fate is unknown, use other means).
