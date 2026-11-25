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
  resolver adjusts the current floor by the change in test declarations
  between the commit that set the current floor and the pending index.
  This preserves the floor's calibration and includes tests applied by an
  earlier commit in the same ticket. The landing check rejects any floor below the
  pinned fold base, and the Swift test runs still enforce the floor.
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
ticket went back to build. The gate fixture checks for both tickets' test
files and the combined floor of 103. The recorded ov-1424 layer conflict also
reconstructs the table with both `landing_update` and `native_hold` assigned
to the engine layer.

Workspace Clippy passed with warnings denied. All 3,409 selected daemon unit
tests passed across the full run and focused retries with the daemon and hook
CLI fixtures available.
