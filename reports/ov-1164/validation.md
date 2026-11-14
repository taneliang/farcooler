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
