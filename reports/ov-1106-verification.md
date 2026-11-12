# ov-1106 acceptance verification

The branch's implementation is in `1c6a658ea`. This follow-up adds regression
coverage for file claims across boards and lowering a published budget while
runs are active. Scheduling remains in Rust; publishing remains an owner action.

| Acceptance | Implementation and evidence |
| --- | --- |
| Published eligibility, budget, and per-step model rotation | `StartPolicy` in `crates/workflow/src/scheduling.rs`; fixed, unlimited, and observed-load budgets; model lists remain definition-pinned. |
| Automatic starts and refill | Runtime scheduling runs in each ready-effect scan. The fake-clock test starts three tickets, holds the fourth, then starts it on completion. |
| File conflicts and blockers serialize | Declared file claims are compared across boards and against active runs' pinned policies. Dependencies and recorded board blockers must be done. Tests cover shared files, independent work, dependencies, and cross-board release. |
| Reviewed tickets join automatically | `auto_queue` queues `WaitingJoin` tickets through the ordinary reducer. The landing integration test issues no Queue command. |
| Fake clock and load | `fake_clock_and_load_start_three_hold_fourth_and_refill_on_finish` and `load_and_known_files_and_dependencies_hold_starts_and_models_rotate`. Missing load samples and samples at/above the threshold hold starts. |
| No production effect cap; configurable budget | Production effect dispatch has no fixed cap. The cap setter exists only in test builds. Publication tests exercise pause, unlimited starts, restart, and lowering a budget without cancelling existing runs. |
| Landing gates leave build capacity | `a_reviewed_ticket_joins_automatically_and_a_landing_gate_leaves_build_capacity` passes with simulated build capacity one; its gate requires another build to run alongside it. |
| Published execution limits | Optional pinned `Limits` controls retries, back edges, send-backs, model count, reworks, attempts, stages, routes, and group membership. Workflow tests exceed the old retry/model/back-edge/send-back defaults. |

Validation on this branch:

- `TMPDIR=/tmp cargo test -p farcooler-workflow`: passed, including legacy
  canonical encoding, replay, and configurable-limit tests.
- `TMPDIR=/tmp cargo nextest run -p farcooler-daemon --lib -E
  'test(workflow_runtime::)' --no-fail-fast`: 277 passed.
- `TMPDIR=/tmp cargo test -p farcooler-store workflow --no-fail-fast`:
  59 passed.
- `TMPDIR=/tmp cargo test -p farcooler-cli workflow --no-fail-fast`:
  14 passed; one existing private-receipt fixture test ignored.
- File-size budget, doc-comment check against the ticket base, swallowed-call
  lint, and `git diff --check`: passed.
- Removing the budget and file-conflict admission guards temporarily makes both
  new regression tests fail. Restoring the guards returns all six scheduling
  tests to passing.

Git/process fixture tests required execution outside the filesystem sandbox.
No live policy was published and no real agents were launched for verification.
Usage and policy JSON examples are in `docs/workflow-scheduling.md`.

Policy-removal follow-up:

- Added a restart regression showing that removing `start_policy` leaves admitted
  runs unchanged, holds pending tickets after completion, and allows a later
  publication to resume admission.
- Strengthened the automatic landing/capacity test by publishing a definition
  without `start_policy` before the reviewed ticket queues. Its admitted
  `auto_queue` policy still joins it without a Queue command.
- Re-ran the workflow crate tests and the daemon workflow runtime suite:
  278 daemon tests passed. Both changed tests also passed together.
- Temporarily replacing absent policy with unlimited admission and selecting
  auto-queue from the current publication made both regression tests fail at
  their respective waiting-ticket and joined-phase assertions. Restored the
  original scheduling implementation afterward; all eight scheduling and
  automatic landing/capacity tests then passed.
- File-size budget, swallowed-call lint, doc-comment check against HEAD, and
  `git diff --check` passed.

Review-return follow-up (Nov 13):

- Read ticket ov-1106, including finding `28ae8987`, through the bundled CLI.
- Before changing admission behavior, ran five regression tests and observed
  all five fail on the reported assertions: one load sample admitted 40 runs
  across two boards instead of one; the second board started beyond the shared
  fixed budget; a tracker-only board performed five scheduling state reads
  instead of zero; refused Start and Queue each grew receipts from two to six
  during repeated scans instead of holding at two.
- A load-limited scan now admits one run total. Every published runner budget
  applies to total active runs across all boards, including runs on boards
  whose policy was removed. Fixed and load constraints apply together.
- The scheduler reads each changed board snapshot once and caches it by revision.
  Tracker-only boards without a policy are skipped. Accepted commands return
  the store's committed state, so later starts, file conflicts, model rotation,
  and queues use current state without reloading aggregates per candidate.
- Refused Starts and Queues back off for one minute. Fake-clock tests verify
  receipt counts stay flat during the delay and a retry occurs at the deadline.
- Additional tests cover combined fixed/load limits, snapshot reuse through
  multiple successful starts, and invalidation after an external completion.
- No production policy was published and no real agents were launched.
- Final validation: `TMPDIR=/tmp cargo nextest run -p farcooler-daemon
  -p farcooler-store -p farcooler-workflow --lib -E
  'package(farcooler-store) | package(farcooler-workflow) | test(workflow)'
  --no-fail-fast`: **994 passed**, including all five regressions and the
  combined fixed/load and cache-invalidation tests.
- `TMPDIR=/tmp cargo test -p farcooler-cli workflow --no-fail-fast`:
  **14 passed**, one existing private-receipt fixture test ignored.
- File-size budget, doc-comment check, swallowed-call lint, and staged diff
  whitespace checks passed.
