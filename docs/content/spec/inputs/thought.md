The clean model is this:

A retry is not “run the RPC again.” It is “re-address the same logical operation.”

Everything else follows from that.

1. The semantic core: single-assignment logical operations

The RPC layer should model a call as a logical operation with a stable identity, not as a sequence of transport attempts.

That logical operation has:
	•	a client-chosen operation identity
	•	a bound method + canonical request + caller scope
	•	exactly one authoritative terminal outcome

A transport attempt is just one way of talking about that operation.

This is the only model that handles the ambiguity you care about. After a disconnect or lost response, the client cannot know whether the previous attempt never arrived, is still running, finished successfully, or finished with failure. The system needs a way to say “these are all attempts to observe or continue the same thing.”

Without that identity, the system cannot distinguish:
	•	“please continue/retry the thing I already started”
	•	from
	•	“please perform a second, new operation that happens to have the same arguments”

Those are not the same. A payment API is the classic example, but the problem is general.

So yes: operation identity is necessary if transparent retry is meant to be correct rather than aspirational.

⸻

2. What “same logical operation” should mean

The identity must bind at least:
	•	caller / principal / tenant scope
	•	target service / method
	•	canonicalized request payload
	•	optionally relevant execution context if it changes semantics

If the same operation ID comes back with a different bound request, that is not a retry. It is a conflict and should be rejected.

That matters because otherwise you can accidentally alias two different logical operations to one identity.

Also important: same payload is not enough to mean same logical operation.

Calling create_invoice(customer=7, amount=10) twice may intentionally mean two invoices. So payload-based dedup is not semantically sound as a general RPC rule. Operation identity must be explicit.

⸻

3. The state machine I would use

I would define a logical operation as moving monotonically through these states.

A. Absent

The server has no admitted record for that operation identity.

This does not mean “the client’s earlier attempt definitely did not arrive.” It only means there is no authoritative server record now.

A retry with the same identity may be admitted here and become the first real execution.

B. Live

The operation has been admitted and is not yet terminal.

Useful substates:
	•	Accepted / Pending
	•	Running
	•	CancellationRequested

The exact internal breakdown is not the semantic surface. What matters is that there is at most one live logical execution owner for that operation ID.

C. Sealed(outcome)

The operation has a final outcome, and that outcome is single-assignment.

Possible terminal outcomes:
	•	Succeeded(result)
	•	Failed(error)
	•	Cancelled
	•	Rejected(...) if you want to distinguish business/validation rejection from general failure

I would keep Rejected only if the API already treats it meaningfully differently. Semantically it is still just a terminal outcome that replays.

D. Expired / Forgotten

This is really a retention boundary, not a business state.

Once the system has forgotten enough state that it can no longer safely interpret the operation identity, it must not silently run the operation again. It should return something like “expired / unknown / cannot safely retry.”

That is critical. Reusing a forgotten identity as though it were fresh breaks the whole model.

⸻

4. What duplicate attempts should do

Given a retry with the same operation identity:

If the operation is Absent

Admit it and start the logical operation.

If the operation is Live

Do not run a second handler instance for the same logical operation.

Instead:
	•	attach the new attempt to the existing operation and wait for the same outcome, or
	•	return “still in progress” and let the client poll / reattach

Either behavior is fine. The semantic requirement is the same: no concurrent duplicate execution for the same operation identity.

If the operation is Sealed(outcome)

Replay the same terminal outcome.

Not “equivalent enough.”
Ideally the same result or same serialized error.
At minimum, the same stable semantic outcome.

If the first attempt succeeded and the response was lost, the retry must not re-run the handler. It must return the already chosen success outcome.

If the first attempt failed terminally, the retry should replay that same failure, not opportunistically try again and maybe succeed. Otherwise the meaning of “same operation” collapses.

If the operation is Expired

Return an explicit expired/unknown error.
Do not turn it into fresh execution.

To run again, the caller must create a new operation identity and knowingly perform a new logical operation.

⸻

5. The crucial distinction: attempt failure vs operation outcome

A lot of confusion disappears if you separate these two things.

Attempt failures

These are failures of a particular delivery/execution attempt:
	•	connection dropped
	•	timeout waiting for response
	•	process died before durable seal
	•	cancellation requested but not yet resolved

These are not automatically logical-operation outcomes.

Operation outcomes

These are the final outcomes of the logical operation itself:
	•	success
	•	business rejection
	•	terminal failure
	•	cancellation with no commit

Transparent retry should be defined over the operation, not over attempts.

So the rule I would use is:

A failure seals the operation identity only if the system has chosen it as the authoritative terminal outcome of the logical operation.

That means:
	•	a transient pre-commit crash does not have to seal
	•	a validation error usually does seal
	•	a post-commit failure usually does seal
	•	an uncertain partial external side effect that cannot be recovered safely means the method should not claim transparent-retry safety in the first place

This is the principled answer to your “should failure seal?” question: sometimes yes, but only terminal failures of the logical operation. Not every failed attempt.

⸻

6. When re-run is acceptable

A same-identity retry may cause execution/resumption/restart only when one of these is true:
	1.	No previous attempt committed any externally visible effect, and the system knows that.
	2.	Previous work was only tentative/internal/recoverable under the operation identity.
	3.	Any external effects already performed are themselves deduplicated or transactional under the same operation identity, so continuing is still semantically the same operation.

A re-run is not acceptable just because:
	•	the client timed out
	•	the connection broke
	•	a cancel was requested
	•	the server process restarted

Those events create uncertainty, not permission.

The real criterion is: can the system still uphold single-assignment semantics for this operation identity?

If yes, continue or restart.
If no, the method was never transparently retryable.

⸻

7. Cancellation and “dropped interest”

This is where many designs get sloppy.

Cancellation should be a request, not rollback magic

Cancellation means: “if this operation has not committed, please try to stop it.”

It does not mean:
	•	“pretend it never ran”
	•	“it is now safe to re-execute”
	•	“any in-flight side effects are undone”

Best race rule

Cancellation competes with commit.

Whichever happens first wins:
	•	if cancellation is observed before commit and the service can guarantee no committed effect, the operation seals as Cancelled
	•	if commit already happened, later cancellation does nothing semantically; the operation seals as the committed outcome

That gives you a clean, unsurprising contract.

Dropped interest is not the same as cancellation

A client disconnecting or no longer waiting should not by itself change the logical meaning of the operation.

The cleanest rule is:
	•	disconnect detaches the observer
	•	explicit cancel requests cancellation
	•	an implementation may generate an implicit cancel request on disconnect, but semantically that is still only a best-effort cancellation request

That distinction matters because otherwise transport flakiness starts changing business semantics.

Retry after cancel

A retry with the same operation identity after a cancel request should reattach to the same operation, not create a new one.

If the operation later seals as Cancelled, retries replay Cancelled.

If the client wants to “try again from scratch,” that is a new logical operation and needs a new operation ID.

That prevents cancellation races from turning into duplicate execution.

⸻

8. Partial side effects: the real fault line

This is the hardest part, and it is where the runtime stops being able to save you.

Transparent retry is only correct if the service can define a meaningful commit point for the logical operation.

Before that point:
	•	no externally visible irreversible effects, or
	•	effects are tentative, rollbackable, or hidden, or
	•	effects are themselves keyed by the same operation identity

After that point:
	•	the operation must have a durable terminal outcome bound to the operation identity

If a handler can do an irreversible external thing and then crash before the system durably knows that this operation committed, you have a semantic hole.

Example shape of the bad case:
	1.	handler sends an email / charges a card / mutates another system
	2.	server crashes
	3.	operation record was never sealed
	4.	client retries
	5.	system cannot know whether to replay, continue, or rerun

That method cannot honestly support transparent retry unless the downstream action itself participates in the same identity discipline.

So the principle is:

Exactly-once-ish retry semantics do not come from transport reliability. They come from making the operation identity authoritative across every effect that matters.

If downstream systems do not participate, the RPC layer alone cannot manufacture correctness.

⸻

9. Crash recovery

Crash recovery is the same issue in slower motion.

For a method to support transparent retry across crashes, the system needs durable recovery of one of these:
	•	a sealed outcome
	•	enough live operation state to resume safely
	•	proof that no commit happened, so restart is safe

If after a crash the system can end up in “maybe the effect happened, maybe not, and I have no durable evidence either way,” then transparent retry is not sound for that method.

This is why the real contract is not “we retry your handler.” It is “we preserve single-assignment meaning for the logical operation across failure.”

⸻

10. What belongs to transport vs operation semantics

A lower layer can do useful things:
	•	reconnect a session
	•	retransmit lost frames
	•	preserve ordering within a session
	•	hide some packet loss
	•	maybe resume an interrupted stream

All of that is good, but it is not enough.

Transport/session reliability can reduce how often ambiguity escapes, but once the client loses definitive knowledge of whether the remote operation committed, the problem is no longer transport-level. It is an operation semantic problem.

So I would draw the boundary like this:

Transport/session layer owns
	•	bytes, framing, ordering, retransmission
	•	connection liveness
	•	session resumption
	•	per-attempt delivery mechanics

RPC operation layer owns
	•	logical operation identity
	•	duplicate suppression/coalescing
	•	single-assignment terminal outcome
	•	cancellation race resolution
	•	retry meaning across disconnected attempts
	•	retention / expiry semantics

That division stays conceptually clean.

⸻

11. The contract service implementers must satisfy

For a method to be marked transparently retryable under this model, the implementer must satisfy these obligations:

1. Define a semantic commit point

There must be a meaningful point at which the logical operation becomes “done” in the externally visible sense.

2. Ensure pre-commit work is safe

Before commit, any side effects must be one of:
	•	absent
	•	rollbackable
	•	hidden from external observers
	•	recoverable under the same operation identity
	•	deduplicated downstream by the same identity

3. Make the terminal outcome durable

Once committed, the operation must have a durable, replayable outcome bound to the operation identity.

If the exact result matters, it must remain recoverable for the retention window.

4. Never let same-identity duplicates execute independently

If the runtime sees the same operation ID again, it must not hand it to the business logic as a second independent operation.

5. Treat cancellation as pre-commit best effort

Cancellation can prevent commit if it wins early enough.
It must not be interpreted as automatic rollback after commit.

6. Propagate the identity to downstream effects when needed

If correctness depends on external calls, those systems need the same identity discipline or equivalent transactional coupling.

7. Respect the retention contract

Operation identity must stay meaningful for an explicit retention window.
After that, the system must fail safely, not silently reuse the identity.

That is the actual cost of transparent retry. Anything weaker is convenience language.

⸻

12. Do you need multiple method categories?

I would expose one semantic model and only one real capability split.

Category 1: transparently retryable

The method satisfies the single-assignment logical-operation contract above.

Internally, it may get there in different ways:
	•	naturally safe rerun
	•	durable dedup + outcome replay
	•	resume from journaled state
	•	transactional commit with operation log
	•	downstream idempotency keys

But those are implementation strategies, not semantic categories.

Category 2: not transparently retryable

The method cannot guarantee single-assignment semantics across ambiguous failure.

For these methods, the system must not pretend that retry is transparent. It must surface uncertainty and require either:
	•	explicit reconciliation, or
	•	a new logical operation with a new identity

I would avoid more categories unless they change the caller-visible contract. “Idempotent,” “replayable,” and “safe to rerun” are useful engineering notes, but they do not need to become separate semantic classes if the externally visible contract is the same.

So my answer is: a single model is sufficient, with an opt-in capability boundary.

⸻

13. Edge cases, explicitly

Response lost after success

First attempt commits, response disappears.
Retry with same ID returns the same success outcome.
No rerun.

Duplicate attempt while original is still running

Second attempt attaches to the live operation or gets “in progress.”
No second execution.

Client disconnects after sending request

Server may continue, or may receive an implicit cancel request.
That does not authorize rerun.
Retry with same ID observes whatever state the same operation is actually in.

Cancel races with success

If cancel wins before commit and no effect is committed: Cancelled.
If success committed first: success replays.
No ambiguous middle should be exposed as “maybe both.”

Handler fails after partial internal progress

If the progress is internal/recoverable under the same op ID, the operation may later continue or restart.
No problem.

Handler fails after irreversible external side effect but before durable seal

This is the semantic cliff.
Unless that external effect is also keyed/transactional under the same op ID, the method is not transparently retryable.

Terminal business failure

Replay it.
A retry of the same logical operation should not turn a validation failure into success.

Crash after commit but before response

Replay the committed outcome after recovery.

Crash before commit, with proof no external effect committed

Retry same ID may safely restart.

Operation record garbage-collected too early

You have lost correctness.
So the contract must specify a retention window, and expiry must fail closed.

⸻

14. The shortest version of the recommendation

I would design the system around this rule:

A transparent retry is a re-attempt to observe or complete one client-named logical operation, which has at most one committed effect set and exactly one authoritative terminal outcome.

That implies:
	•	caller-generated operation identity is required
	•	same-identity duplicates never run independently
	•	sealed outcomes replay, including terminal failures
	•	cancellation is a race against commit, not rollback
	•	pre-commit crashes do not necessarily seal
	•	post-commit ambiguity must be eliminated by durable operation state
	•	methods that cannot uphold this must not claim transparent retry support
	•	expiry must fail closed, not silently become fresh execution

That is the model I’d want if correctness and conceptual clarity come first.
