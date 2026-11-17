# ov-1317: TaskList keeps its field name

Tag 3 is `next_page_after` again, with every Rust reader, writer, and fixture
updated. The tag, optional bytes type, and paging behavior stay the same, so
protobuf clients compiled with either spelling still exchange the same bytes.
Older daemons ignore `page_after`; newer clients still accept their unpaged
reply. Restoring the name also keeps JSON-facing tooling compatible.

## Why main CI missed it

The committed Canary baseline at 4a0fee927 predates TaskList tag 3. CI ran
`proto-lint.py --channel canary`, so both adding `next_page_after` (60ddb56c71)
and renaming it to `next_after` (0009b4798, carried by ov-1302) appeared additive
against that snapshot. Only Canary's shipping workflow used `--since-baseline`,
which compared against the intermediate main commit and correctly refused it.
The landing gate reads CI's wire run steps, so it inherited CI's omission.

CI now checks out full history and runs `--channel canary --since-baseline`.
The landing gate reads and runs that same step. A regression test in
`scripts/landing-gate-test.py`, itself run by both gates, constructs a baseline
before paging, commits the original cursor, and then renames it. It proves the
old baseline-only command exits 0 and the history-aware command exits 1 with
the precise rename diagnostic. It also pins the CI command and checkout depth.
Removing `--since-baseline` from CI was tested: the new test fails; restoring
it makes the test pass.

The protocol's board-document isolation test previously treated any occurrence
of "page" in a field name as a board-document dependency. It rejected the
restored pagination cursor. Its narrow exception now verifies that this exact
TaskList field stays bytes at tag 3, while still rejecting board-page message
types and all other page-named fields in task/plan payloads.

## Baseline recovery

The ticket establishes that no Canary build after 8628aecc0 exists, so the
`next_after` spelling never shipped. History includes both spellings, meaning
restoring the name alone would then fail against 0009b4798. The original
recovery anchored at candidate restoration commit
`a8481042643e693926dd6a93a162330a0e03b194`. Review caught that the landing fold
cherry-picks with `-x`: fresh checkouts contain the rewritten commits, so that
candidate-only anchor blocks the lint again.

Fix round 1 anchors at the existing main tip before this repair,
`63374831f3903403f60dcc35c6f1f1b04a1e383a`. The repaired snapshot is unchanged;
it requires `next_page_after` and is compatible with the last shipped baseline.
Only the known unshipped interval is excluded from history. Landing preserves
this existing main ancestor, and every subsequent proto remains checked. The
baseline comment and docs/releasing.md record this manual recovery exception;
the header is a recovery anchor, not evidence of a shipped build.

The new landing-gate regression makes a shipped wire, an unshipped rename,
and candidate restoration/recovery commits. It folds both candidates with
`cherry-pick -x` and clones only main over `file://`. Both candidate objects
are confirmed absent. The actual lint passes with the existing-main anchor,
fails with the former candidate-only anchor's missing-history diagnostic,
and refuses a new rename of tag 3. This test runs in per-push CI and the
landing gate through the existing landing-gate-test.py step.

## Validation

- Canary compatibility with history: passes (wire.log).
- Proto lint self-tests: 107 passed (wire.log).
- Landing-gate unit tests: 7 passed, including the rename regression.
- Negative check: dropping the history flag makes the regression test fail.
- Affected Rust selection: **253 passed**, 0 failed, 19.666 s test time
  (rust.log). Includes all protocol tests; CLI TaskList fixtures and paging;
  client board reads; daemon task operations, starts, workspace operations;
  and ov-1302's long-history, archived-tail and 1,500-task frame/paging tests.

Rust command (foreground, four build/test jobs):

```sh
TMPDIR=/tmp SDKROOT=$(xcrun --show-sdk-path) NEXTEST_TEST_THREADS=4 \
  cargo nextest run --jobs 4 \
  -p farcooler-protocol -p farcooler-cli -p farcooler-daemon -p farcooler-client \
  -E 'package(farcooler-protocol) or test(task_ops::tests) or test(task_starts::tests) or test(workspace_ops::tests) or test(tasks::) or test(board_reads) or test(plan_tests) or test(plan_agent_dispatch_tests) or test(page_tests) or test(rep_pane) or test(workflow_runtime::process_logs::tests)'
```

The first Rust run inside the sandbox could not initialize daemon git fixtures
or bind test sockets; it also exposed the board-document test issue described
above. After correcting that assertion, the selection passed outside the
sandbox. The landing-gate unit tests likewise passed outside the sandbox,
which permits their process supervisor to inspect its child processes.

No full gate, cargo fmt, or swift-format was run. Build and test concurrency
was limited to four. Local validation uses system Python because the first
python3 on PATH is an incompatible executable; compiler temp files use /tmp.

## Fix round 1 validation

- Canary lint with history: passes; 107 proto lint self-tests pass.
- Repaired snapshot versus the last shipped baseline (4a0fee927): compatible.
- Landing-gate unit suite: 8 passed, including fold/fresh-clone regression.
  Used system Python first on PATH and ran outside the sandbox because the
  fixture supervisor needs `ps` to inspect its own child processes.
- Canary baseline recorder regression suite: 41 passed.
