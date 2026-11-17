# ov-1317: TaskList field-name compatibility

## Fix round 3 (latest)

Restored `TaskList.next_page_after` on tag 3 in the schema, every Rust reader,
writer, and fixture. The bytes and pagination algorithm are unchanged. A
protocol descriptor test pins the original name, tag, and encoded bytes. The
board-page isolation test exempts only this task pagination cursor by message,
name, and tag; it still forbids BoardPage types and every other page reference.

The previous recovery preserved `next_after`, which passed lint by skipping the
original field-name history instead of restoring compatibility. That approach
was rejected in the latest ticket finding and is superseded here.

## Why main CI missed it

The last shipped Canary baseline (4a0fee927) predates tag 3. Main CI checked
only that snapshot, so both adding `next_page_after` and renaming it to
`next_after` looked additive. Canary alone checked intermediate main history
with `--since-baseline`, so its shipping job caught the rename after CI passed.
The landing gate consumes CI's wire steps and inherited that omission.

The existing per-push CI fix is retained: full checkout history and
`proto-lint.py --channel canary --since-baseline`. The landing gate consumes the
same steps. Its regression shows the old snapshot-only check accepts a rename
and the history-aware command rejects it.

## Recovery and original history

The recovered snapshot now requires `next_page_after`, as first added in
60ddb56c71. Direct comparisons of the corrected schema against that original
commit and the last shipped baseline both pass. Changing the corrected schema
back to `next_after` is rejected by the recovered baseline.

The history anchor remains existing main commit
63374831f3903403f60dcc35c6f1f1b04a1e383a, at the end of the unshipped interval.
It survives the landing fold's `cherry-pick -x`. The snapshot deliberately
differs from that commit's proto: it restores the original field name instead
of preserving the unshipped rename. This distinction is recorded in its header
and docs/releasing.md. Neither spelling of tag 3 shipped; only the mistaken
history interval needs exclusion, while the original name remains required.

The recovery regression models the shipped schema without tag 3, the original
addition, the unshipped rename, restoration, and baseline recovery. Before
recovery, it verifies that only the renamed historical version objects to
restoration. It folds the repair and recovery using `cherry-pick -x`, clones
only main (without candidate objects), and verifies the lint passes. It also
rejects a candidate-only anchor and a subsequent rename of the restored field.

## Validation

- Canary history-aware proto lint: passes.
- Proto lint self-tests: 107 passed.
- Page-layer lint and self-test: pass. The new cursor case was seen red
  before the exact declaration allowance; negative cases still reject a wrong
  message, wrong tag, BoardPage type, and an extra page field on the same line.
- Landing-gate unit suite: 8 passed, including strengthened history assertions
  and fresh-clone fold recovery (landing-round3.log).
- Negative CI mutation: dropping --since-baseline makes the rename regression
  fail; the restored workflow passes (rename-red-round3.log).
- Original-history comparisons: corrected schema is additive against both
  60ddb56c71 and 4a0fee927; reverting to next_after reports the rename error.
- Affected Rust selection: 254 passed, 0 failed, 12.260 s test time
  (rust-round3.log), including ov-1302 long-board paging, CLI/client reads, and
  the new original-name/tag/bytes protocol test. Four build and test jobs.

Affected Rust command:

```sh
TMPDIR=/tmp SDKROOT=$(xcrun --show-sdk-path) CARGO_BUILD_JOBS=4 NEXTEST_TEST_THREADS=4 \
  cargo nextest run --build-jobs 4 --jobs 4 \
  -p farcooler-protocol -p farcooler-cli -p farcooler-daemon -p farcooler-client \
  -E 'package(farcooler-protocol) or test(task_ops::tests) or test(task_starts::tests) or test(workspace_ops::tests) or test(tasks::) or test(board_reads) or test(plan_tests) or test(plan_agent_dispatch_tests) or test(page_tests) or test(rep_pane) or test(workflow_runtime::process_logs::tests)'
```

Earlier logs in this directory describe superseded attempts, not the current
schema. No full gate, cargo fmt, or swift-format was run.
