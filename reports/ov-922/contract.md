# ov-922 core/store handoff

Baseline d625aaeb9. Package implements a validated finite forward sequence (1–16 stages), with optional
review, repeated checks, selected/every-stage approval, and bounded evaluator rework
to a configured stable earlier candidate-producing Build/Action stage ID.
Validation also rejects forward evaluator paths without a candidate producer. Coding and no-Git checklist definitions
are examples, not engine policy. Named Action/ Evaluate bindings support noncoding
sequences: Action produces a candidate; Evaluate binds that candidate and may reject
into rework. OperationSpec pins the exact adapter configuration. These are durable intents, not a runtime.
The tracker must be enqueued with an exact immutable checklist candidate; both
approval and the no-op result bind that same revision.

`farcooler_workflow::step(&State, &Input) -> Result<Transition, Refusal>` is pure.
`Definition::validate` bounds the sequence and validates unique stage IDs, generic bindings, attempts and
rework limits. `Definition::pin` and `digest` use versioned canonical typed JSON
and SHA256. Items retain their own definition/policy pins. No in-place item edit
or pin migration API is supplied. Dependencies name existing items only, so
insertion cannot form cycles. Cancelled/failed prerequisites never satisfy them.

`Input` includes caller-generated stable ID, source/sequence, workspace,
expected revision, authority epoch, supplied UTC milliseconds and `Grant`.
The daemon MUST construct grants from authenticated admission, never deserialize
an untrusted actor's claimed grant as authority. Controller can enqueue, cancel,
and reconcile uncertainty. Named decision grants approve the exact run/pin/candidate
at the current revision; semantic review is a separate decision. Operation grants
bind effect/run/fence. Informational ACK is intentionally absent from this API.
The runtime must enforce mandatory policy/grants, canonical resource identities,
install writer ownership and adapter receipt authenticity before admission.

`Store::workflow_apply(&Input)` admits or records a stable refusal, returning a
`Receipt`. It transactionally writes accepted input, new aggregate, outbox,
resource ownership and receipt. Duplicate ID+identical typed payload returns the
original receipt even after revision moves; changed payload returns an error.
A rejected receipt consumes no workflow sequence/revision and remains deduplicated.
`workflow_state`, `workflow_events`, `workflow_outbox` are read-only recovery APIs.
`workflow_claim_effect` and `workflow_admit_effect` persist scheduling metadata;
neither performs external I/O. Claims bind revision, epoch, owner, lease generation
and persistent resource generation. Only expired pre-admission claims may be reclaimed.
A successful admit is a single CAS; repeated admission returns false. Once admitted,
expiry never grants replay. The runtime must reconcile and submit Uncertain or an
authenticated result. Accepted observations require admitted status; explicit Resolve
requires uncertain status. Reducer replay does not replay scheduling metadata.
Outbox `active` means currently referenced, NOT eligible for execution. Inspect
phase and dispatch_state; uncertain work can only reconcile. The runtime must add
scheduler/authentication/adapter integration before any real I/O.

A pending effect has deterministic identity, run, epoch, fence, resource, pins,
candidate, kind and pinned OperationSpec (adapter version, exact arguments,
sorted environment). Changing settings changes the definition pin and effect payload;
no mutable default command may be resolved at retry time. Fixture adapter versions
must be replaced/adapted explicitly for real harness/Git integration. Adapter version
implementations and injected runtime context must preserve these immutable semantics. Results need a matching authenticated operation grant and
nonempty immutable manifest/receipt/candidate identifiers. Check/review/land/CI
must match the Build candidate. Runtime adapters must validate exact base, check
environment, remote ref, landed SHA and CI provenance; this crate cannot authenticate
Git/provider facts. `SafeFailure` is allowed only after proving no unsettled effect
or live writer remains. It can retry within the stage budget. `Uncertain` keeps
ownership and requires explicit correlated resolution. Cancellation emits a stop
intent with a new attempt fence and holds ownership until `Stopped { operation, receipt }`;
the authenticated stop receipt must establish process quiescence AND settlement of
the original external operation, not just successful delivery of a stop request; cancellation
does not assert that an accepted external effect was undone.

The reducer fence is per attempt. Store reservation generations are separately
monotonic per resource, survive release as tombstones, and must be passed to adapters.
An epoch/generation in SQLite alone cannot stop a running external writer: adapters
must enforce fencing or report uncertainty. Canonical resource identities and actual
install writer ownership remain runtime obligations.

Store marks superseded outbox intents inert in the same transaction; historical
payloads/events remain immutable. Superseded admitted operations retain uncertain
status until a correlated stop reconciliation settles them. The store must not be
used to infer that cancellation undid an accepted external mutation. Resource ownership covers admitted approval/review/running/uncertain/cancelling items.
Blocked items hold no reservation; dependency-ready activation checks resource
availability sequentially in stable item-ID order, admitting at most one waiter per
resource. The store stamps `Input.busy_resources` from other-workspace reservations
(restricted to relevant item/enqueue resources), ignoring caller-provided values.
Accepted event payload/hash includes that snapshot for pure replay; receipt payload/hash
retains the original request for exact duplicate identity. Occupancy is bounded separately
from the 64 KiB request. Completions can commit while cross-workspace dependents remain
blocked. A controller `Advance` command re-evaluates these waiters after external release;
the future runtime must schedule it from resource-release/recovery observations.
Cancellation of blocked work remains supported. Cross-workspace admission
conflicts produce a stable refusal and can be retried with a new command ID. Cross-workspace conflicts reject
atomically. There is no owner-data conversion, legacy dual write, runtime call
site, external effect execution or production selector.

The registry append uses `Older::Refused` and registry-derived schema version.
Rebase/integration must recheck registry order. Full main/integration, adapter,
client, migration-conservation and owner-data release checks remain parent gates.
