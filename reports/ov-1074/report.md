# ov-1074: adopted Provision and recovery

## Findings

The original hook-file refusal was fixed by `3f06cebd3`, already in this branch's base.
`provision_adopted` calls `refuse_adopting_changes`, which uses the shared working-tree
reader and hides only untracked hook paths. `verify_tree` and `verify_candidate` use
`tree_changes`, which also skips only those untracked paths and exact temporary hook
files. Git itself excludes ignored files. Tracked hook edits and other uncommitted
files remain changes. Provision retains early refusals in its binding-specific
`Unstarted` receipt and reconciles them as `SafeFailureBecause`, with a sentence.

The Nov 13 ticket history described ov-1095 and ov-1098 as stuck. Read-only inspection
in this session found that ov-1098's adopted Provision has a matching Git settlement
and a Passed journal (candidate `573b541cc723250eae21382b4dc310d1ee00686e`), and the item
has reached Build. Its receipt is
`wf1:8923477142e5ca45d5c6bb057e9d87490ab083c9cc99e816ee452ecd09bc1061.json`.
Its Git log lists only the untracked `.codex/hooks.json`, then successfully reads HEAD
and the expected branch. No live state or hook files were changed during this work.

A remaining recovery defect is in `stop_coding`: cancellation reconciles candidate
validity to establish quiescence. A completed Provision can consequently remain
Uncertain if someone switches its worktree to another branch. The completion receipt
already proves its bounded Git operation returned. Cancellation now authenticates
that receipt against the original binding and records Stopped without demanding the
current tree remain a valid candidate. It does not change branches, remove the tree,
discard files, relaunch Provision, or claim the original candidate passed. Missing
or foreign completion receipts still leave cancellation Uncertain.

## Safe recovery

For the original hooks-only failure, install a daemon containing `3f06cebd3` (already
on main). Its normal recovery sweep rereads the exact existing Git completion receipt,
checks the repository, branch and clean tree past the untracked hooks, and resolves
Provision as Passed, which immediately schedules Build and can launch an agent in the
adopted branch. Use this route when continuing the run is intended; cancel first if
it should be abandoned. No mutation command is needed to resume. ov-1098's retained Passed journal
and subsequent Build provide evidence that this route actually works. Do not delete
hooks, rewrite engine records, or use push settlement for Provision.

If a completed Provision's tree has since moved and the run should be abandoned,
install this change, read `farcooler workflow get <workspace>`, then use the inspected
run/pin/revision/epoch and the caller's next sequence:

```sh
farcooler workflow cancel <workspace> <ticket> \
  --run <run> --pin <pin> --command-id <unique-command-id> \
  --revision <revision> --epoch <epoch> --sequence <next-sequence>
```

Wait until the workflow reports Cancelled. A closed ticket can then be reopened:

```sh
farcooler workflow reopen <workspace> <ticket> \
  --command-id <another-unique-command-id> \
  --revision <fresh-revision> --epoch <epoch> --sequence <next-sequence>
```

Read a fresh snapshot before each new mutation; reuse the exact command identity and
arguments only for a lost reply. Cancellation across a daemon restart is covered by the new
regression. It requires an authentic completion receipt. A Provision with no such
proof remains Uncertain: neither retry, reopen, send-back nor `settle --not-landed`
can safely invent proof that its old operation ended. `workflow settle` handles Push
only. No recovery command above was run on the live runner.

## Other questions from the original brief

The no-hooks native build failure was diagnosed and fixed separately in `9775ca12c`
(ov-1075), already landed. On macOS, tokio's peer credential lookup also requested a
PID after the hook client disconnected, producing ENOTCONN; replying to that closed
client could also produce EPIPE. `coding_native::peer_uid` now uses `getpeereid`,
which preserves the connected UID, and the native turn tolerates a failed empty
reply. The earlier scratch-HOME Claude reproduction and real hook capture are in
`.claude/agent/reports/ov-1074/` in the main checkout. This session does not claim a
new signed-in agent reproduction or write to the owner's agent configuration.

The red-main follow-up was implemented by `e3007f85e` (ov-1076). `await_base` now
accepts a completed Failed CI result as well as Passed; it still waits for pending
CI. The folded candidate must pass the batch's gate and post-push CI. The retained
`a_batch_on_a_base_whose_ci_failed_folds_gates_and_lands_and_ci_after_the_push_decides`
regression exercises both successful repair and failing post-push CI. No new follow-up
is needed for that already implemented behavior.

## Validation

- New cancellation regression: observed red before the fix (`Uncertain` instead of
  `Cancelled`), then green. The same regression checks missing and foreign-generation
  receipts stay Uncertain without replaying Provision, and preserves the moved branch
  and the person's ignored file.
- Hook exclusion mutation: removing the `tree_changes` filter made the adoption
  service test fail, naming `.codex/hooks.json` and `.cursor/hooks.json` as changes.
  Restoring the filter made it pass. The fixture now also contains ignored output;
  its genuine-change case still fails Provision with a sentence and keeps the file.
- `cargo test --offline -p farcooler-daemon --lib workflow_runtime -- --test-threads=4`:
  **273 passed, 3 ignored**. This includes native hook/cancellation recovery and the
  existing red-main batch regression.
- `cargo test --offline -p farcooler-daemon --test workflow_service_over_socket workflow_service -- --test-threads=2`:
  **18 passed, 1 ignored**.
- Both ticket regressions passed under scratch HOME `/tmp/ov1074-home-0rjkm858`,
  whose `.gitconfig` sets `init.defaultBranch = master` (1 cancellation regression
  and 1 adoption service regression).
- `scripts/file-size-budget.py` and `git diff --check`: passed. Staged doc-comment
  check: passed.

Cargo commands ran through `scripts/heavy --light` with four build jobs. Ignored tests
are existing external/private-fixture or child-harness entries; no test was newly
ignored. All temporary hook-exclusion mutations were restored before green runs.

No app source changed (net Swift/Kotlin lines: 0/0). Commit SHA is supplied by the
commit containing this report and the final response; no push is performed.

## Acceptance verification on the adopted restart

The restart already contains the hook exclusion (`3f06cebd3`) and settled-Provision
cancellation (`26428803b`). This pass extends the socket regression to distinguish
an ordinary untracked file, a person's `.codex/config.toml` beside the hooks, a staged
`.codex/hooks.json`, and an edited tracked `.codex/hooks.json`. Every case must fail
at Provision, preserve the changed contents, and save `SafeFailureBecause` with the
same refusal sentence shown on the item. Only exact untracked hook paths are exempt.

Fresh verification:

- Removed the `adapters::tree_changes` hook exclusion temporarily: the socket
  regression failed because the hooks-only tree was Failed instead of Done,
  explicitly naming `.codex/hooks.json` and `.cursor/hooks.json`. Restored it fully.
- The expanded socket regression then passed, and passed again with scratch HOME
  `/tmp/ov1074-master-home` and `init.defaultBranch = master` (five adoption cases).
- `cargo test --offline -p farcooler-daemon --lib provision -- --test-threads=2`
  under the same scratch HOME: **4 passed**. This covers cancellation after restart,
  missing/foreign settlement receipts, a tree changed before reconciliation, and
  Provision object-download failure/concurrency.
- `git diff --check`, `scripts/file-size-budget.py`, and the doc-comment check
  against HEAD passed. No wire or app types changed; net Swift/Kotlin lines: 0/0.

Filtered Cargo runs used `scripts/heavy --no-lock` with one build job, after the
shared load gate remained busy; the lane rules permit targeted tests freely.
No full suite was rerun in this pass. The commands in Safe recovery were checked
against the installed CLI's help; no live recovery command was executed.

## Landing rebase (Nov 14)

Rebased the two outstanding ov-1074 commits onto `origin/main` at `ed6a0555f`
with `git rebase --no-autostash` after preserving the generated, untracked
`.codex/hooks.json` outside the worktree. The tracked and untracked tree was clean
when the rebase started. The only conflict was concurrent test additions at the
end of `coding_tests.rs`: retained main's rerun, legacy-base, and process-stop
regressions followed by this ticket's Provision cancellation regressions.
`git range-diff` confirms the ticket patches changed only their insertion context.
The reducer, workflow starter, coding verbs, and group tests match main exactly;
no features were added.

Post-rebase validation (no new tests or behavior changes):

- `cargo test --offline -p farcooler-daemon --lib provision -- --test-threads=2`:
  **6 passed**, including main's second-run and legacy recovery regressions.
- `cargo test --offline -p farcooler-daemon --lib workflow_coding -- --test-threads=2`:
  **52 passed**, under scratch HOME `/tmp/ov1074-master-home` with
  `init.defaultBranch = master`; includes the merged tests and native cancellation.
- Targeted `workflow_service_over_socket` adoption regression: **1 passed** under
  that same scratch HOME, covering hooks-only adoption and all four dirty cases.
- `git diff --check`, `scripts/file-size-budget.py`, and
  `scripts/doc-comment-check.py --base origin/main`: passed.

Used system Python because the PATH Python cannot execute on this host. The broad
runtime command was stopped while queued behind other gate jobs, before it built
or tested. Targeted commands ran through `scripts/heavy --no-lock` with one build
job and debug info disabled, as lane rules permit. No full suite was rerun.
Net Swift/Kotlin lines remain 0/0; no wire types changed. No live recovery was run.

## ov-1150 replay onto main

Replayed the three outstanding commits onto `origin/main` at `a6e065c71`.
They applied without conflicts; range-diff shows the three patches unchanged.
The existing main changes remain in place.

With Cargo's target directory in this worktree, the offline targeted tests passed:

- `cargo test -p farcooler-daemon --lib provision -- --test-threads=2`: 6 passed.
- `cargo test -p farcooler-daemon --lib workflow_coding -- --test-threads=2`: 52 passed.
- The `workflow_service_over_socket` adopted-tree regression: 1 passed.

`git diff --check`, the file-size budget, and the doc-comment check against
`origin/main` also passed. No live recovery was run.
