# ov-1317: TaskList wire recovery

TaskList keeps `optional bytes next_after = 3`, as ov-1302 named it to obey
ov-269's board/page boundary. All Rust readers, writers, and fixtures use that
name again. The earlier exception in the protocol's page isolation test is
removed. Tag, type, and paging behavior are unchanged.

## Why main CI missed it

The last shipped Canary baseline (4a0fee927) predates tag 3. Main CI checked
only that snapshot, so both adding `next_page_after` and renaming it to
`next_after` looked additive. Canary alone checked intermediate main history
with `--since-baseline`, so its shipping job caught the rename after CI passed.
The landing gate consumes CI's wire steps and inherited that omission.

Per-push CI now uses full checkout history and
`proto-lint.py --channel canary --since-baseline`. The landing gate runs the
same step. The retained regression in `scripts/landing-gate-test.py` constructs
a pre-paging baseline, adds tag 3, and renames it: the old check passes, and
the history-aware check fails with the rename diagnostic. It also asserts
CI's history flag and checkout depth.

## Baseline recovery

Neither tag 3 spelling shipped; the last shipped baseline has no tag 3.
The recovered snapshot uses `next_after`, anchored at existing main commit
63374831f3903403f60dcc35c6f1f1b04a1e383a, after ov-1302's rename. This follows
the documented recovery procedure and needs no protocol exception. The header
records a recovery anchor, not a shipped build. Its snapshot is checked against
the last shipped baseline.

The recovery regression now models that exact sequence: a shipped wire without
tag 3, an unshipped addition and rename, and baseline recovery alone. It folds
the recovery using `cherry-pick -x`, clones only main, and verifies the lint
passes without candidate-only objects. It rejects a candidate-only anchor and
a subsequent rename of `next_after` to `next_cursor`.

## Validation

Current validation logs use the `round2` suffix. Earlier logs describe the
superseded restoration and are retained only as historical evidence.
