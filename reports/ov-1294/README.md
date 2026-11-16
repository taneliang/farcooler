# ov-1294: CI Mac native tests stop waiting on Swift

## Result

No workflow change ships. The wall time is held by the Mac budget, not by the
`needs` edges, and every edge arrangement that stays inside the budget leaves
the same critical path. The three trial runs below were measured with a
temporary branch-push trigger, which is removed again; `ci.yml` and
`scripts/ci-macos-capacity.py` are byte-identical to `31646caf5`.

## The budget that requires the edges (acceptance 4)

`scripts/ci-macos-capacity.py` models the account: five Macs at once (the
GitHub-hosted macOS limit), four for landing CI and one reserved for background
work, and 20 jobs in all. It passes today at exactly `landing 4 Macs`, `20/20`
total jobs. Measured on the four baseline runs (43797643365, 43795002152,
43789217612, 43774779075): peak Macs in one run was 4 every time, peak jobs 11
to 12.

On the trial branch the pool was visibly shared: with a main run alongside,
`Swift` and the native archive started 1.8 to 4.4 minutes after the run was
created, and `Rust native tests` and `iOS UI (smoke)` waited 5 to 8 minutes for
a slot. That queueing is the capacity limit showing up, not a dependency.

## Why moving edges does not help

The Mac jobs add up to about 71 job-minutes (Swift 11, Swift tests 12, native
archive 7, iOS 6, Rust on macOS 8, native shards 2 x 9, iOS UI smoke 9). Over
four slots that is about 18 minutes before any setup or queueing, against a
baseline median of about 22. The current chain (Swift tests 12, then the shards
9) and the best rearrangement the antichain check admits (smoke waits for Swift:
11 + 9; Rust on macOS waits for Swift tests: 12 + 8; shards wait for iOS:
7 + 9) both end at about 21. The trial made the tail the smoke job instead of
the shards and removed nothing.

The edges tried, and what the checker said, are in the commit history of the
branch (`6b4cacea7`): dropping any Swift edge from the native shards without
ordering `ios` and `Rust (macos)` against them makes five Macs possible, and
the capacity self-test rejects it.

## Android and Web

Android is not on the critical path. Its chain (unit tests 3, release build
13.6) is 17.5 minutes; the Mac tail is 21 to 22. Android UI `rest` ran in 10
minutes instead of 14.3 when allowed to start with `first`, but that needs one
more job slot and the job budget is full (22 against 20 until `Tunnel (Go)` was
ordered after `Wire`), for no change in wall time. Web (10 to 13 minutes) is
under the Mac tail too, so splitting it shortens no run. Not done.

## Trial runs (all on the branch, edges rearranged)

| Run | Wall | Notes |
| --- | --- | --- |
| 43804704834 | 36.7 | Red. Main CI ran alongside. Native archive 13.6, native shards started at +20.5 and +23.8 (Mac queue). `Rust tests (1/4)`: `workflow_service_admitted_ci_recovers_original_bundle_after_reconfiguration` failed. iOS UI smoke 18.2, failed after a 7.5 minute simulator boot |
| 43808938513 | 28.4 | Cancelled at the smoke job's 20 minute limit (simulator boot again). Without it the last job finished at about 21 (Rust tests 20.1, then Relay) |
| 43812224858 | 29.1 | Red: `Swift (AgentKit + Mac tests)` failed. Smoke waited for Swift and ended last |

Baseline walls: 22.6, 21.6, 19.6, 28.4 (median about 22). The three trial walls
are not a clean comparison: two hit simulator problems and all three shared the
Mac pool with other runs. They show no gain; they do not show a loss that the
edges caused.

## Not changed

Every job and test still runs: no job was added, removed or changed.
