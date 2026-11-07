# ov-963 result

Design: `reports/ov-963/design.md`. Private synthetic fixtures and scratch homes only. No owner database, app or daemon was touched.

## What now exists

- **Commit.** `workflow.cutover` `Commit` and `farcooler workflow-cutover commit`, served only in recovery. `closing::commit` holds one kernel proof across the closed copy, the independent restore, the permit and the mapping.
- **Closed backup.** `<db>.cutover/closed-<n>/`, carried by Finish into `-resolved-<attempt>`. Verify and Release refuse if the backup the commit named is lost or changed.
- **Back to service.** A successful Finish or Recover ends the recovery daemon with exit 75, so launchd or systemd restarts it into ordinary service.
- **Exits kept.** A schema Release would refuse now stops before Commit. Verification runs inside the commit transaction, so a failure rolls back whole and stays frozen, where Recover works.
- **Old build (R3).** `JoinedOwners::acquire` refuses while another process has the file open, and `inspect` lists the pids. Nothing is signaled.
- **Device told (M6).** `DomainError::CutoverHeld` reaches a standalone stdio device.

## Defect found by the real crashes

A process killed mid-transaction leaves a nonempty journal with no header. SQLite neither replays nor removes it, so `refuse_sidecars` refused every later stage, Recover included. The constructed states from ov-940 could not show this. `clear_inert_journal` now removes a journal whose first byte is zero, after SQLite has had its chance to roll back. Mutant `killed-journal-stays` pins it.

## Evidence

- `crates/daemon/tests/a_cutover_survives_every_crash.rs`: 29 boundaries, each a real process killed by SIGKILL, on raw schema 60 (26 boundaries, no hold path) and a current runner (all 29). Each reopens through real main, stream, stdio, `Service` and `Store`, then goes back and forward from a repeat crash.
- `reports/ov-963/mutants.py`: 37 mutants, all built and all red, covering every boundary (`mutants-results.json`). The script refuses to run if a boundary has no mutant.
  - The first run missed 3. One was a wrong expected list. One needed raw60, since a current source already has the floor. One was a real gap: a row checked that `baseline.db` existed, not that it was whole. The check was strengthened and all three reran red.
  - That re-run preceded a later one-line guard in the test (`dir.exists()`, needed once artifacts are renamed), so the guard itself was not mutated.
- `reports/ov-963/real-cli.py` (`real-cli.json`): the real `farcooler` CLI and a production-feature `farcoolerd`, Prepare to ordinary service. `FARCOOLER_TEST_CUTOVER_KILL_AT` was set throughout and ignored. Exit 75 seen.
- `reports/ov-940/old-reader.py`, rerun: the pinned registry-60 Store at d625aaeb opens a raw fixture and refuses the file this build's Prepare leaves with `NewerData`.
- Store 605 passed, 3 ignored. Daemon cutover, recovery, refusal and writer-join filters pass, apart from the failure below. Matrix file 6 passed in 106 seconds. Clippy is clean on everything this lane touched.
- The new tests were not shown red before the code, because the API did not exist. The mutants are the red evidence, as in ov-940.

## Not proved

- **Durability.** SIGKILL leaves the page cache alone, so sync order is not crash-proven. A power-loss test was not run.
- **Restore after Commit.** Still unbuilt. A Verify that fails after Commit leaves the runner in recovery with its data intact and no in-product exit. The in-transaction check means this needs the file to change after Commit.
- **Registry 61 to 64.** `preflight` refuses them. Real installs were not read, so the owner-copy step must confirm which registries exist.
- **Remote owner (R4).** A remote owner can run Prepare and not Recover. Unchanged.
- **A second pty.** An orphan on a new pty counts as a person to `recovery_seat`. The tests rely on this, and an agent on the same login could do the same. The review bounded this impact; it needs a ruling before release.
- **Linux.** The `/proc` scan and the orphan test ran only on macOS here.

## Red on this base, not from this lane

None of these files' failing lines are in this diff.

- `people::actor_tests::every_write_that_names_an_actor_is_listed`, as ov-940 reported.
- `file-size-budget`: `crates/store/src/store.rs` is 1 line over. This lane does not touch the file.
- Clippy: `crates/daemon/src/workflow_runtime.rs:473` and `crates/store/src/workflow_overlay/tests.rs:205`.
- `copy-lint`: 5 Android title-case strings.

## What changed for the ov-936 rehearsal

Rerun at this branch's tip: the example reached `committed`, with zero fake sends, pushes and Enters. For its next full run:

- `JoinedOwners::acquire` now refuses on another process's descriptor, so the rehearsal fails if anything holds its file.
- `engine_commit` verifies inside its transaction.
- `CommitPermit` has two evidence kinds. The rehearsal keeps `Quiescence`.
- The raw60 fixture moved to `farcooler_store::testing::write_source60`.
- `testing::commit_for_tests` is gone. `released_transfer_for_tests` remains.
