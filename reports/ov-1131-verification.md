# ov-1131 verification

Brought in all six commits from `origin/main..ov-1106-engine-auto-start` in
chronological order. Resolved conflicts with main's newer CI-policy work by
preserving both CI policies and the published scheduling/execution limits.
The cherry-picked commits are signed.

Read ov-1131 and ov-1106 through the installed canary CLI, and inspected the
parent ticket's Codex transcripts and newest available review receipts. The
parent branch already includes the repeated-scan load-gap fix, runner-wide
budget accounting, revision-cached snapshots, refused-command backoff, and
constraints from policies published on boards without aggregates.

The remaining bypass was runtime restart: the last successful start timestamp
was only in memory. Added a fake-clock/load regression at load 5.999 with a
limit of 6, covering repeated scans, completion, restart, another board, clock
rollback, missing/high samples, and exactly one new admission after the gap
with an acceptable sample. Before the fix it failed at:

```text
restart must retain the runner-wide allowance at 99
```

The scheduler now recovers the latest accepted automatic start from the durable
event log once on its first scan. Completed and archived runs retain that
history. Refused starts and unrelated runtime events cannot move the deadline;
a store regression checks that distinction. Later scans retain the existing
cached scheduler behavior. The owner-configurable gap and all earlier
acceptance coverage are preserved. Scheduling documentation describes recovery.

Validation:

- `PATH=/opt/homebrew/bin:$PATH TMPDIR=/tmp cargo test -p farcooler-workflow
  --no-fail-fast`: **151 passed**, including all integration tests and legacy
  encoding/replay checks.
- `PATH=/opt/homebrew/bin:$PATH TMPDIR=/tmp cargo nextest run
  -p farcooler-daemon -p farcooler-store -p farcooler-workflow --lib
  -E 'package(farcooler-store) | package(farcooler-workflow) | test(workflow)'
  --no-fail-fast`: **1,009 passed**, including both new regressions, earlier
  scheduling tests, automatic landing/capacity tests, and full store libraries.

- `PATH=/opt/homebrew/bin:$PATH TMPDIR=/tmp cargo test -p farcooler-cli
  workflow --no-fail-fast`: **14 passed**, one pre-existing private-receipt
  fixture test ignored.
- `PATH=/opt/homebrew/bin:$PATH TMPDIR=/tmp cargo clippy
  -p farcooler-daemon -p farcooler-store -p farcooler-workflow --lib
  -- -D warnings`: passed.
- File-size budget, swallowed-call lint, doc-comment check, and staged/working
  diff whitespace checks: passed.

Git-backed fixture tests require host execution because sandboxed Git
invocations are refused.
No production policy was published and no real agents were launched.

## Continuous-sweep review return

Read both tickets again and the latest ov-1131 review transcript
(`rollout-2026-11-13T11-22-46-01a1d2ed-dc3b-79b1-9a5f-afed2e789c88.jsonl`).
It identified a service-loop starvation defect: `sweep_loop` reused the
timestamp passed to `pump` for every scan while any effect remained running.
The scheduler's direct fake-clock tests advanced time, but the continuous loop
never did, so a long gate could freeze the load gap indefinitely.

Added a regression through the actual continuous sweep with a held in-process
effect and Tokio's paused clock. Before the fix it failed at:

```text
a running effect must not freeze admission time
```

Continuous sweeps now advance the supplied timestamp using monotonic elapsed
time for scheduling, effect admission/recovery, and observations. One-shot
fixture pumps retain their explicit timestamps. The regression checks repeated
scans before 60 seconds, exactly one new admission at each of two successive
gaps in the same sweep, and the final observation's advanced timestamp. A
permissive load threshold isolates the loop timer from host load; the existing
near-limit fake-load, restart, combined-budget, and missing/high-sample tests
remain in the affected suite.

Fresh validation, with `PATH=/opt/homebrew/bin:$PATH TMPDIR=/tmp`:

- `cargo nextest run -p farcooler-daemon -p farcooler-store
  -p farcooler-workflow --lib -E 'package(farcooler-store) |
  package(farcooler-workflow) | test(workflow)' --no-fail-fast`:
  **1,010 passed**, including the continuous-sweep regression.
- `cargo nextest run -p farcooler-daemon --test workflow_service_over_socket
  --test workflow_commands_over_socket --test workflow_draft_approval
  --no-fail-fast`: **27 passed**, one existing ignored test skipped.
- `cargo clippy -p farcooler-daemon -p farcooler-store -p farcooler-workflow
  --lib -- -D warnings`: passed.
- File-size budget, swallowed-call lint, doc-comment checks, protocol lint,
  copy lint, visual-token lint, and diff whitespace checks: passed.

## Acceptance audit on resumed build

Read ov-1131 and ov-1106 again using the installed Canary CLI. All six parent
commits and both ov-1131 fixes are already on this branch; no second
cherry-pick is needed. The latest ov-1131 review receipt
(`wf1:bf10aea8fb92e1a3c7ec9bd29c1dafdc6fc25430837c47895c155782e7f93095.json`)
passes candidate `c750d2b7c`. Its preceding rejection is addressed by the
continuous-sweep regression above.

Inherited acceptance coverage:

| Item | Implementation and regression coverage |
| --- | --- |
| 1: Published eligibility, budget, step models | `StartPolicy`, definition validation, and `load_and_known_files_and_dependencies_hold_starts_and_models_rotate` |
| 2: Automatic starts and capacity refill | `published_policy_is_pumped_without_start_commands_and_can_pause_and_resume` and `fake_clock_and_load_start_three_hold_fourth_and_refill_on_finish` |
| 3: Serialize known conflicts and blockers | `file_claims_serialize_across_boards_without_holding_independent_work`, pinned-claim republishing coverage, and dependency checks |
| 4: Reviewed tickets join automatically | `a_reviewed_ticket_joins_automatically_and_a_landing_gate_leaves_build_capacity`, including publication removal after admission |
| 5: Three starts, fourth held, then refill | `fake_clock_and_load_start_three_hold_fourth_and_refill_on_finish` |
| 6: Published fixed, unlimited, and load budgets | `Budget`, runner-wide combined constraints, lowering-budget tests, and empty-board publication coverage |
| 7: Landing gates release build capacity | `a_reviewed_ticket_joins_automatically_and_a_landing_gate_leaves_build_capacity` |
| 8: Published execution limits | `Limits`, `published_limits_override_retries_back_edges_and_model_counts`, and `published_send_back_limit_can_exceed_the_old_maximum` |
| ov-1131 repeated-scan burst | `near_limit_burst_keeps_load_allowance_across_scans_and_restart` at load 5.999/6, plus the continuous-sweep timer regression |

The latest landing receipts report a conflict applying `bc537cfec` to the
landing base in `crates/workflow/tests/routes.rs`; they report no additional
scheduler review finding. This audit does not claim a successful landing.

Fresh validation on the resumed build, with `PATH=/opt/homebrew/bin:$PATH
TMPDIR=/tmp`:

- `cargo nextest run -p farcooler-daemon -p farcooler-store
  -p farcooler-workflow --lib -E 'package(farcooler-store) |
  package(farcooler-workflow) | test(workflow)' --no-fail-fast`:
  **1,010 passed**, including both ov-1131 regressions.
- `cargo test -p farcooler-workflow --no-fail-fast`: **151 passed**.
- File-size budget, swallowed-call lint, doc-comment check against
  `origin/main`, and diff whitespace check: passed.

This resumed build changes only this verification record. The implementation,
regressions, and earlier failing-before-fix evidence remain in the existing
branch commits.
