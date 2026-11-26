# ov-1462 — Baseline conflicts at landing

## Engine records

The window is **2026-11-12 05:59:44 through 2026-11-26 05:59:44 SGT**. I read
the local engine database in read-only mode and counted distinct paths per
conflict event. For queue preflight, the source is `workflow_events` commands
with `RefreshJoin.result.Conflict.paths`. For group folds, the sources are
`workflow/receipts/*.verb.json` with `Execute: Fold` and `label: conflict`, and
`workflow_events` outcomes with `FoldConflict.files`, joined by effect ID to
avoid counting the same failure twice. Older fold outcomes used a
label without file paths in the event, so the Git verb facts supply those paths.
A fold's diagnostic replay can log the same conflict twice in its Git trace.

| Baseline or floor | Queue preflight | Group fold | Total | Evidence |
| --- | ---: | ---: | ---: | --- |
| `apps/macos/test-floor.txt` | 3 | 0 | **3** | Event revisions 6013, 6501, 6523. ov-1424 twice and ov-1430 once. |
| `scripts/daemon-layers.json` | 0 | 1 | **1** | Event revision 6596 and its fold fact. ov-1424. |
| `scripts/daemon-layers-counts.txt` | 0 | 0 | **0** | Event revision 6355 is ov-1410's gate rejection for the stale `agents -> service` ceiling (22 recorded, 21 found). |

Those were the only baseline paths in the conflict records. The AgentKit
floor appears in gate failure output during the window.

## Resolution

- Both Swift floors retain their numeric format. When a pick conflicts, the
  resolver adds the incoming commit's floor increase to the current floor.
  Git supplies the picked commit's parent as the base for that increase.
  This preserves the reviewed number of cases for parameterized tests and
  works when the ticket adds its tests in an earlier commit. A side below
  the conflict base requires manual resolution. The landing check rejects any
  floor below the pinned fold base, and the Swift test runs enforce the floor.
- The daemon layer table retains its JSON format. The resolver reconstructs
  the table from Git's three versions, combining independent module ownership
  and policy entries. Two different assignments for the same module, two
  different changes to one policy entry, or an invalid layer table still
  require a person to resolve the conflict.
- The daemon layer counts come from the same scanner as the gate. In a
  conflict the fold can lower each ceiling to the combined tree's count,
  bounded by both sides' ceilings. The normal gate still checks computed
  counts against those ceilings and compares changed ceilings against the
  pinned fold base and fetched main.
- Queue preflight and group fold use the same pick logic. Only regular
  content conflicts in the supported baseline files are eligible. A
  conflicting code path still uses the existing send-back route.

## Verification

The original two-ticket floor test failed before the fix because the second
ticket went back to build. The gate fixture now checks for both tickets' test
files and the combined floor of 111, with one ordinary test and ten cases from
one parameterized test. The floor regression reproduced the old resolver's
result of 102 when the parameterized test was picked second. The recorded
ov-1424 layer conflict also reconstructs the table with both `landing_update`
and `native_hold` assigned to the engine layer.

Workspace Clippy passed with warnings denied. All 3,409 selected daemon unit
tests passed across the full run and focused retries with the daemon and hook
CLI fixtures available.

## Landing gate send-back

Group 7079 ran the layer check against `aff2e5d0c`. The gate receipt
`wf1:10157f66ae037cb43fa99b599e08bce111734534feb1bcfc6592b73b5d0b8d85.json`
records both ov-1462 and ov-1441 as group members. The layer check passed on
the ov-1462 candidate `b081346c7`. It failed on the recorded fold with nine
`engine -> board` edges against a ceiling of eight. The extra edge is the
`crate::plan_run::run_of` assertion added to `workflow_runtime/coding_tests.rs`
by ov-1441 (`a775f2f70`, folded as `9bbadd178`).

In a scratch copy of the recorded fold, replacing the assertion expression
with `item.failed_reason().as_deref()` preserved the comparison with the
reason returned by the workflow API. The same layer check then passed with
645 recorded edges across 55 layer pairs and the original ceiling of eight.
The board layer already tests the plan's failure reason in `plan_run_tests.rs`.
The assertion repair belongs on ov-1441, which owns the line and its test.

On the send-back, Clippy passed with warnings denied. The landing-gate
Python suite, floor checks, daemon layer checks, and file-size budget check
passed. The daemon run passed 3,401 tests, including the shared-floor fold
and the code-conflict send-back. Eight tests failed on the first run and
passed in isolated retries. The builder-priority retry used the shell's
inherited niceness of 10 so the builder could lower its priority further.
Eighteen tests were ignored by the harness.

For the parameterized-test follow-up, workspace Clippy and all 28 landing-gate
Python tests passed. The daemon run passed 3,404 of 3,409 selected tests.
The remaining five passed with native Python and the engine's inherited
niceness of 10. The focused rerun also passed the shared-floor group gate
and the code-conflict send-back.
