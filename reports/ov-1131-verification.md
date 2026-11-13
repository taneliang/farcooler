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
