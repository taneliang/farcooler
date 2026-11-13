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
