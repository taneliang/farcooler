# ov-1209: next group gates during CI

A successful push transfers its group from the shared landing resource to a
private resource for CI and later steps. The transfer and the next group's
acquisition happen in the same store transaction, preserving effect admission
and generation fences.

The next base step can take that pushed head while its group's CI is pending.
Its durable base receipt names the previous group, including its workspace.
The next push reads that receipt and waits for the previous group to succeed;
its pinned CI definition still judges all configured workflows and retries.
A failed previous group returns `base_moved` before anything is sent, so the
waiting group takes its base again, folds again, and runs its gates again.
The existing ability to land a repair on an already-red base remains.

No gate command, CI command, workflow list, or CI success criterion changed.

## Fix round 1: recovery during the CI barrier

The push writes and fsyncs a binding-specific pre-send record before its checks
and CI barrier. Before either transport can send, it writes and fsyncs separate
send intent. Reconciliation with no settlement resumes only when the pre-send
record matches the full binding and no send intent exists. Missing legacy
records, damaged or foreign records, and attempted sends remain uncertain.
An interrupted pre-send wait gets a new effect fence without spending an
attempt or extending its original deadline.

The pipeline regression now reconciles the pending second push, drops the live
runtime futures, reopens the runtime, and releases the predecessor's CI. Both
red and green paths finish, with exactly two actual push-process receipts and
event replay matching the persisted state. Before this fix, the test failed
at reconciliation with `push has no settlement; whether it landed is unknown`.
The reducer's restart test also covers repeated pre-send interruptions and
checks that the attempt and deadline are retained.

Fix-round validation (with `TMPDIR=/tmp` and the same temporary target directory):

- `cargo test -p farcooler-workflow`: all tests and doctests passed.
- `cargo nextest run --lib -p farcooler-daemon -E 'test(workflow_runtime)' --test-threads 8`:
  297 passed, including the restart regression, damaged/foreign pre-send records,
  partial send intent, re-entry prevention, and existing lost-push settlement tests.
- `cargo clippy -p farcooler-workflow -p farcooler-daemon --lib --tests -- -D warnings`: passed.
- File-size budget, doc-comment check against `HEAD`, and `git diff --check`: passed.

An earlier runtime-suite run was invalidated by a concurrent rebuild replacing
its executable; the complete clean rerun above passed without failures.

## Acceptance evidence

`next_group_gates_during_ci_and_rebases_after_red` uses two separate landing
groups and a private bare Git remote. The first CI remains pending until the
second group has finished its gate and reached push. After another 300 ms,
the test checks that the remote still has only the first pushed commit, then
releases the first CI as either red or green.

- Red: the second group enters base twice and eventually lands after regating.
- Green: the second group enters base once and then lands.
- Both paths replay their events to exactly the persisted state.
- The controlled CI cannot finish before the second gate, demonstrating that
  the gate work overlaps the CI wait instead of adding to it.

Before implementing the change, the red path failed with:

```text
assertion `left == right` failed: red previous CI returns the waiting group to base
  left: 1
 right: 2
```

Run the regression with:

```sh
TMPDIR=/tmp cargo test -p farcooler-daemon --lib next_group_gates_during_ci_and_rebases_after_red
```

## Validation

All commands used `TMPDIR=/tmp`; build artifacts were kept in
`CARGO_TARGET_DIR=/tmp/ov1209-target`.

- `cargo nextest run -p farcooler-workflow`: 155 passed.
- `cargo nextest run --lib -p farcooler-daemon -p farcooler-store -p farcooler-workflow -E 'package(farcooler-workflow) or (package(farcooler-store) and test(workflow)) or (package(farcooler-daemon) and test(workflow_runtime))'`:
  354 passed, two pull-request fixture failures under concurrent load.
- Serial rerun of those two failures (`a_create_that_hangs_after_it_worked_is_judged_by_the_lookup_not_the_timeout`
  and `a_line_gh_printed_to_stderr_does_not_hide_its_answer`) plus the final
  pipeline regression: all three passed. The pipeline regression also passed
  in the concurrent suite.
- `cargo clippy -p farcooler-workflow -p farcooler-store -p farcooler-daemon --lib --tests -- -D warnings`: passed.
- `scripts/file-size-budget.py`, `scripts/doc-comment-check.py --base HEAD`,
  and `git diff --check`: passed.
