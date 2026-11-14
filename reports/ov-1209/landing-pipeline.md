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
