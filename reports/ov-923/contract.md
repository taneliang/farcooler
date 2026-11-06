# ov-923 isolated runtime API

Source baseline: signed bd87202f0fa7628c8905e5179e6fd5d8c408002c.
Source implementation pin: signed f52442374. Only an explicit isolated fixture runtime is constructed; no Service/main selector.

`workflow_runtime::Runtime::create(root)` creates its own private repository,
SQLite database and receipt directory. `open(root)` resumes that exact fixture.
Both hold one nonblocking lifetime writer flock. Resources are canonical identities
derived from that repository, never caller aliases or arbitrary paths.
`controller()` and `decision_session(controller, principal)` are nonserializable,
instance-bound local capabilities. A future transport must mint them only after
its existing authentication/policy admission. Requests contain command identity,
expected revision and command, never a Grant. Controller requests can enqueue or
cancel, decision sessions can approve or submit semantic review; result/Resolve
authority is internal and only accepts the runner's exactly bound receipt.
No public raw Input/Grant/result admission API.

`pump(now_ms)` enumerates persisted workspaces, schedules Advance for blocked work,
claims/admit-CAS eligible current Running/Cancelling effects, then executes a closed
adapter version. `run(shutdown)` owns this loop; notifications/consumer ACKs are
absent from eligibility. Pre-admission claims can expire; admitted work is only
reconciled. Missing or mismatched receipts remain Uncertain with ownership held.

The finite fixture adapters exercise existing Git worktree/revision primitives,
a supervised deterministic terminal harness/check/reviewer, local compare-and-swap
landing and existing CI response parsing. They have immutable OperationSpec versions
and exact arguments. There is no arbitrary adapter registry, provider or push.
Every receipt binds the full effect, resource generation, definition/policy pins,
run/fence, exact candidate, base and fixture iteration. Raw logs survive failures.
Durable accepted receipts reconcile after restart without re-execution. A stop
requires original-operation settlement and process quiescence; absence of a receipt
is never proof of either. Unknown original operations remain uncertain.

First-slice limitation: trusted deterministic fixtures only. This is not an
untrusted-code sandbox or real provider authorization. Real harness/provider smoke,
all-client integration, conservation/removal/cutover and main gates remain parent
release requirements. Independent review and final evidence will freeze this API.

## Source and caller obligations

- `crates/daemon/src/workflow_runtime.rs:30`: nonserializable instance-bound local
  capabilities; `Request` rejects unknown fields. `controller()` is trusted local
  construction, not remote authentication. Keep the runtime handle behind the
  existing authenticated service boundary when adding a protocol caller. Admission
  rejects operation/Resolve commands from callers. Caller retries must preserve
  request fields and the supplied `now_ms`, matching the core typed-input identity.
- `workflow_runtime.rs:78`: canonical private root, lifetime writer flock,
  alias checks, full immutable base SHA and one derived repository resource.
- `workflow_runtime.rs:164`: persisted aggregate discovery, occupancy-change/recovery
  Advance, claim/admit CAS, receipt-only admitted recovery. `active` alone is not
  execution eligibility. `run` owns the bounded pump and shutdown loop.
- `workflow_runtime/adapters.rs:5`: full binding includes workspace, entire immutable
  effect, reservation generation, fixture iteration and base. The private receipt
  directory is trusted local storage; it is not proof against malicious same-UID
  processes. There is no public serialized receipt/result ingress.
- `workflow_runtime/adapters.rs:115`: actual retained guarded Git worktree primitive;
  Build uses a deterministic owned process, not a signed-in agent. Check/reviewer
  surround evaluation with exact HEAD/clean-tree verification. Landing is a local
  ref CAS; no push. CI parses a local exact-SHA fixture using retained CI code.
- `effect_runner.rs:17` and `landing_job.rs:440`: one shared process-group runner,
  deadline/kill/reap/quiescence checks; the existing landing gate retains its
  inherited environment. Fixture children get fixed environment plus run/fence,
  resource generation, definition pin and candidate context.
- `workflow_runtime/adapters.rs:190`: stop can settle only a never-admitted original
  or an original with correlated runner settlement. An ambiguous original holds
  ownership. Cancellation never claims an accepted mutation was undone.
- `Store::workflow_workspaces`, `workflow_notices` and StoredEffect's generation
  field are additive read/recovery APIs; no migration or legacy caller changed.
  `notices(workspace, after_revision)` supplies deterministic delivery IDs. Delivery
  reads/cursors cannot approve or block an effect. Subscription/reconnect policy
  belongs to the next caller package, not this runtime fixture.

`coding_definition(review)` supplies pinned finite fixture specs; definitions can
adapt approval principals/progression, omit review, or use the no-Git tracker. This
is a closed fixture capability set, not a promise that all core-validated arbitrary
stage arrangements can execute in this isolated repository. Unsupported operation
versions/arguments/environment reject before enqueue.

Uncertainty without an exact runner receipt deliberately has no automatic replay
or arbitrary controller-supplied success override. Recovery tests supply a delayed
trusted journal internally; a real-provider reconciliation mechanism is not claimed.
The fixture holds one writer and serializes effects; command/cancel admission occurs
between bounded effects. Real concurrent harness cancellation and provider-specific
reconciliation remain integration work before production authority is connected.

## Review-correction API (supersedes loop/base descriptions above)

Correction source pin: `74e8ccc8380da15c55531124a46b7094b205f779`. Original implementation line numbers above refer
to f52442374; use this correction source for current locations.

The review-fix stack adds `command_handle(&Controller) -> Result<CommandHandle>`.
Mint it before starting `run`; clones share a bounded 32-slot local inbox. Its
async `submit(&Controller, Request, now_ms)` and `decide(&DecisionSession, Request,
now_ms)` return the existing durable Receipt and use the owning runtime's existing
instance/principal/command/revision checks. Queue admission awaits capacity; only
SQLite acceptance is durable. On lost replies/restart, retry the identical request
and time. A dropped caller does not revoke accepted work. No raw Grant or result
is accepted. `state(workspace)` is read-only and does not trigger a scheduler scan.
`reconcile()` explicitly wakes local bound-journal reconciliation, supplying no
result or authority override. Handles/capabilities do not survive Runtime reopen.

`run` drains commands between effect sweeps and continues immediately while work
progresses. Once idle it waits on inbox, shutdown or the nearest actual expired
pre-admission claim deadline. Startup discovers persisted work; resource release
still triggers the existing occupancy-driven Advance. There is no idle polling,
notice/ACK dependency or receipt-file timer. A trusted runner that writes a delayed
receipt must signal reconcile (or restart the runtime); foreign/mismatched receipt
bytes still fail exact binding. Shutdown/cancel remain between bounded effects.

Provision claims ownership, then writes and fsyncs an immutable per-item base pin
before admission. It samples the current landed ref only for a new item's first
Provision. All that item's effects, rework, stop validation, durable journals,
recovery and landing CAS reuse the pin. Reopen never resamples an admitted pin;
legacy admitted v1 items without a pin retain their original creation base.
The pin is private trusted fixture storage, not remotely supplied configuration.
This permits consecutive coding items to land descendants in the same isolated
repository without weakening the exact old-SHA update-ref CAS.

Touched source paths: workflow_runtime.rs (inbox/run/lease wake),
workflow_runtime/adapters.rs (durable base propagation), workflow_runtime/tests.rs
(focused regressions), and store/src/workflow.rs (read-only lease_until exposure).
No other source files or production callers change in this correction package.
