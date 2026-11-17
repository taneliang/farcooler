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

## Baseline recovery

The ticket establishes that no Canary build after 8628aecc0 exists, so the
`next_after` spelling never shipped. History includes both spellings, meaning
restoring the name alone would then fail against 0009b4798. Following the
manual recovery procedure in docs/releasing.md, the Canary snapshot and its
history anchor now name a8481042643e693926dd6a93a162330a0e03b194, the committed
restoration. This is a manual recovery anchor, not evidence of a shipped build;
the snapshot carries a comment recording that exception. Compatibility with
the previous shipped snapshot was checked before advancing it.

## Validation

- Canary compatibility with history: passes (wire.log).
- Proto lint self-tests: 107 passed (wire.log).
- Landing-gate unit tests: 7 passed, including the rename regression.
- Negative check: dropping the history flag makes the regression test fail.

No full gate, cargo fmt, or swift-format was run. Build and test concurrency
was limited to four. Local validation uses system Python because the first
python3 on PATH is an incompatible executable; compiler temp files use /tmp.
