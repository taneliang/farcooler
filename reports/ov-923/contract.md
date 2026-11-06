# ov-923 runtime API (implementation in progress)

Source baseline: signed bd87202f0fa7628c8905e5179e6fd5d8c408002c.
Only an explicit isolated fixture runtime is constructed; no Service/main selector.

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
