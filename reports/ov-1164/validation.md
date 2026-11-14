# ov-1164: gate retry requires process quiescence

Continues local `ov-1158-engine-honest-waits` at `5b656a46c`.
The existing ticket branch had diverged, so the predecessor was merged while
preserving the branch's starting commits. Landing definitions are unchanged.

## Cause and fix

The `run_coding` error handler interpreted every Run I/O error as `SafeFailure`.
That included owner-record write failures after spawn, unproved leader reaping,
and a process group whose disappearance could not be established.

The supervisor now carries explicit retry safety with its error. Only failures
before spawn and errors returned after observing whole-group disappearance are
safe. Other process errors become refusals and remain `Uncertain`, retaining
ownership reconciliation. Ledger cleanup cannot mask such a refusal as a safe
I/O failure. Existing launch artifacts also require reconciliation.

Recovery cannot assume that a missing owner record means nothing was launched:
if the durable launch log exists without an owner record or process fact, the
outcome stays uncertain. An identifiable owner can be reconciled through the
existing process-group reaper before a bounded stage retry is authorized.

## Regression evidence

Both new tests failed against the predecessor before the fix:

- `an_unproved_gate_settlement_refuses_retry_until_ownership_is_settled` injected
  an I/O settlement error after recording a real gate process. The process was
  still alive, but the result was `SafeFailure` instead of `Uncertain`. The test
  retains and reaps its own child, verifies that no retry is queued while the
  old process lives, and permits the retry only after ownership settlement.
- `a_gate_owner_write_failure_keeps_recovery_uncertain` failed the atomic owner
  write after spawn. It also returned `SafeFailure` instead of `Uncertain`.
  Recovery must refuse to infer quiescence from the absent owner record.

The initial sandboxed run could not invoke Git; regression evidence above came
from the same test run with normal local subprocess access. With the fix, the
76-test gate subset passed, including pre-launch ENOSPC bounded retries, deleted
fold-tree recovery without split, and a gate recovered after daemon restart.

All builds use the worktree's own `target/`. Tests clear inherited `FARCOOLER_*`
variables, set `TMPDIR=/tmp`, put system tools first on PATH, and use four build
jobs and four test threads, as in ov-1158's validation.

The first full run exposed a further ordering issue in the repeated-invocation
assertion: preparation could fail before the existing launch artifact was
checked. The test now deliberately invalidates the worktree's Git backpointer
while the retained gate lives, then restores it before reconciliation. This
failed deterministically with `SafeFailure` instead of `Uncertain`. The Run
launch-evidence guard now precedes all preparation, so an old writer's tree is
not touched and preparation errors cannot authorize a retry of that invocation.

Final full-suite testing passed every landing regression, including the
strengthened existing-launch test. It encountered the native terminal recovery
flake already recorded in ov-1158's validation:
`workflow_native_review_vanished_server_keeps_output_and_recorded_verdict` stayed
at `agent-review / Uncertain` instead of reaching
`orchestrator-review / WaitingReview`. The isolated rerun passed (1 test).
The affected library suites were then rerun together; results follow below.

## Final verification

- Workspace clippy, `cargo clippy --workspace --all-targets -- -D warnings`:
  **PASS**, exit 0 on the final source.
- All ov-1158 landing regressions and both ov-1164 regressions: **PASS** in the
  final full run and the library rerun. This includes split/wait patience,
  interrupted CI on the pushed SHA, missing-tree recovery, cherry-pick ENOSPC,
  pre-launch ENOSPC bounded retries, and engine-tree removal protection.
- Full five-crate run (workflow, store, daemon, CLI, core), including integration
  tests and doctests, under `scripts/tmux-leak-check.py`: exit 101. Two targets
  failed: the native recovery test above and the pane-resize integration test
  `a_grown_pane_is_announced_before_its_repaint_after_a_respawn_onto_a_new_tty`
  (`/dev/ttys005 was never handed out again`). Other targets passed.
- Five-crate library rerun: exit 101, solely the same native recovery test;
  daemon library reported 3,085 passed, 1 failed, 13 ignored. Workflow, store,
  and core libraries passed.
- Serial resize rerun under the leak wrapper:
  `cargo test -p farcooler-daemon --test a_resized_pane_says_so -- --test-threads=1`:
  **PASS**, 3 tests, exit 0.
- Serial native recovery rerun under the leak wrapper:
  `cargo test -p farcooler-daemon --lib workflow_native_review_vanished_server_keeps_output_and_recorded_verdict -- --test-threads=1`:
  **PASS**, 1 test, exit 0.
- File-size budget, doc-comment check, and `git diff --check`: **PASS**.
- No process/tmux leaks were reported by the wrappers.

Status: **DONE_WITH_CONCERNS**. The gate retry fix is verified and committed.
Every failing test passed in isolation, but acceptance item 3 does not have an
unqualified clean parallel full-suite run. The native recovery flake predates
this ticket; the resize test also depends on global tty allocation. Neither
terminal behavior nor their tests were changed for this gate-ownership fix.

## Fix round 1: isolate tests that depend on tty reuse

The review correctly rejected the earlier exit-101 runs as acceptance evidence.
The two failing fixtures depend on the host's shared PTY-number pool:

- The vanished-review test kills its private tmux server, waits for the old
  tty to have no processes, then reopens the runtime. Another PTY test can
  reuse that number before reconciliation. `cancel_interrupted_native` then
  correctly refuses with `native vanished pane still has processes`, leaving
  the item Uncertain. The existing busy-tty test explicitly requires this
  conservative behavior, including for a tty reused since the crash.
- The resize fixture's `take` opens up to 256 PTYs to reacquire the old tty.
  If another test takes that number first, opening more PTYs cannot recover it.
  The assertion `was never handed out again` is a fixture isolation failure.

The nextest configuration already runs the resize tests exclusively. The
vanished-review test now also requires every scheduler slot, inheriting that
rule in the CI profile. This isolates it from all other tests in that runner,
not merely from the other native tests. Plain libtest does not read nextest's
configuration, so CONTRIBUTING.md now requires `--test-threads=1` for full
plain cargo runs containing the daemon. Neither runner should overlap another
PTY test run on the host. Production ownership checks and gate retry behavior
are unchanged; the gate regressions' fail-before-fix evidence above still holds.

### Final verification for fix round 1

The full four-crate cargo run on the final source **passed, exit 0**:

```sh
python3 scripts/tmux-leak-check.py -- cargo test \
  -p farcooler-workflow -p farcooler-store -p farcooler-daemon \
  -p farcooler-cli --no-fail-fast -- --test-threads=4
cargo clippy --workspace --all-targets -- -D warnings
```

Clippy also **passed, exit 0**. Both commands used the worktree's own target,
four build jobs, TMPDIR=/tmp, system tools first on PATH, and cleared inherited
FARCOOLER_* variables as described above. No process/tmux leaks were reported.

This complete run did not reproduce the intermittent fixture failures: the
daemon library passed 3,086 tests (13 ignored), including the vanished-server
review, the busy-tty refusal, both ov-1164 regressions and the ov-1158 landing
regressions. All three resize integration tests passed in that same run.
CLI, store and workflow unit/integration tests and all doctests passed too.
The new nextest rule addresses the fixture's scheduling premise independently
of whether a particular parallel libtest run happens to win the tty race.

The affected scheduling check also **passed, exit 0** under the CI profile:

```sh
python3 scripts/tmux-leak-check.py -- cargo nextest run --profile ci \
  -p farcooler-daemon \
  -E 'binary(a_resized_pane_says_so) or test(workflow_native_review_vanished)' \
  --status-level all
```

All 5 selected tests passed: vanished-server recovery, busy-tty refusal, and
all three resize cases. No leaks were reported. File-size budget, doc-comment
check, and `git diff --check` passed on the final changes.

Status: **DONE**. This clean full-suite run supersedes the earlier
DONE_WITH_CONCERNS result; acceptance item 3 now has a complete exit-0 run.


## Fix round 2: missing process identity is not quiescence

The newest review found a separate unsafe recovery path in `Owner::reap`:
`started_at` collapsed a failed `/bin/ps` invocation or empty output into an
empty string. A live leader with an empty recorded identity, or a nonempty
record whose current lookup failed, was then treated as a different process.
Recovery returned SafeFailure while the old gate group was still alive.

Start identity lookup now returns an error for execution failure, unsuccessful
status, invalid UTF-8, or empty output. Owner writes propagate that error after
spawn, keeping the existing conservative post-launch failure classification.
Recovery checks whole-group disappearance first. A legacy empty/whitespace
identity refuses recovery while the group remains; a failed current lookup
propagates uncertainty. Only a readable, nonempty differing identity establishes
PID reuse. Leader-probe errors other than ESRCH also refuse settlement rather
than authorizing a signal based on an unproved absence.

New regression evidence, before the production fix:

- `a_gate_with_missing_start_identity_refuses_retry_until_its_group_is_gone`:
  **FAIL**, SafeFailure instead of Uncertain with a live gate and empty saved identity.
- `a_gate_with_unreadable_start_identity_refuses_retry_until_its_group_is_gone`:
  **FAIL**, SafeFailure instead of Uncertain when the live gate's current identity
  lookup is unavailable (task-local injection, no global environment mutation).
- `a_gate_owner_write_requires_a_readable_start_identity`: **FAIL**, owner write
  accepted an unavailable identity and persisted an empty receipt.

The pre-fix `a_gate_` run reported 23 passed, 3 failed, exit 101. An initial
sandboxed attempt could not invoke Git; the regression failures above came from
normal local subprocess access. Both recovery regressions reopen the runtime,
keep the old gate alive while recording its uncertain outcome and unchanged
Run count, then explicitly kill/reap that exact child and verify group
disappearance permits the stage retry. They clean up before their assertions,
including against the unsafe implementation. The first fixed focused run
exposed an incorrect fixture observation mode (Resolve on a still-Running item);
the fixture now uses Observe first, then Resolve after proven disappearance,
matching production dispatch. The other 38 landing tests passed in that run.

### Final verification for fix round 2

The complete required run on the final source **passed, exit 0**:

```sh
python3 scripts/tmux-leak-check.py -- cargo test \
  -p farcooler-workflow -p farcooler-store -p farcooler-daemon \
  -p farcooler-cli --no-fail-fast -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
```

Clippy also **passed, exit 0**, after the final fixture correction. All builds
used this worktree's own `target/`, four build jobs, TMPDIR=/tmp, system tools
first on PATH, and cleared inherited FARCOOLER_* variables. Plain cargo ran
serially to honor the existing PTY fixture isolation guidance. No concurrent
PTY test suite was started by this agent; no process/tmux leaks were reported.

The daemon library passed **3,089 tests, 0 failed, 13 ignored**, including all
three new regressions, both earlier ov-1164 regressions, the ov-1158 landing
regressions, vanished-server native recovery, busy-tty refusal, zombie-group
recovery, and recycled-PID protection. All three pane resize integration tests
passed in this same invocation. Workflow, store (655 library tests passed,
3 ignored), CLI, all integration suites, and all doctests also passed.
File-size budget, doc-comment check, and `git diff --check` passed.

Status: **DONE**. Missing identity no longer authorizes a gate retry. The new
tests demonstrate fail-before-fix behavior and permit retry only after the old
group is proven gone. Acceptance items 1–3 have passing final-source evidence.
