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

## Fix round (review: `independent-review.md`)

- **B2, fixed.** A recovery daemon answers every ordinary method with `CutoverHeld`, the sentence the stdio path already used. A stage refusal now carries a short sentence on the wire: the holder's pid when a process has the file open, otherwise a pointer to `farcooler workflow-cutover inspect`. The raw store text stays in the daemon log. `main_serves_only_recovery` now asserts the exact sentence; the old `CapabilityUnsupported` goes red.
- **B3, fixed.** `a_hot_journal_is_rolled_back_by_sqlite_before_clearing_deletes_anything` builds a real hot journal, and requires the bytes after `clear_inert_journal` to equal SQLite's own rollback of the same image. The review's delete-any-journal mutant, with `settle_journal` removed, goes red. A row-count oracle did not catch it, because the half-written pages sat beyond the root.
- **B1, investigated, not built.** `installs.md`: no Preview or Stable build ever shipped; Canary shipped registries 28 to 57 and the installed app is at 60. Two options for a stepping stone, with a recommendation. The orchestrator rules.
- **Ruling, done.** Commit writes `journal/commit-requested-N.json` (pid, start, tty, session leader) before it does anything. The manager skill says agents never run `workflow-cutover`. The skill's line cap moved from 344 to 346 for it. Two matrix rows now name the journal entry they lack, not the whole directory.
- **M1, fixed.** A live Prepare, held or refused, stops the daemon with exit 75. The test that pinned exit 0 now pins 75, and goes red without the flag.
- **M2, fixed.** The two cutover write connections set `fullfsync` and `checkpoint_fullfsync`. A store test reads both back and fails without them. Power loss is still unproven.
- **Left, named.** M3 (pause updates mid-attempt: needs a product decision on the Sparkle side). M4 (three safety steps with no red mutant: still not counted as proven). M5 (the inert journal is deleted, not kept). M6 and M7 (harmless).
- **Not rerun.** The 37 recorded mutants. The matrix, the store cutover tests and the recovery tests ran green after the changes.

## Last fix round (HEAD after this commit)

- **B1, built.** Prepare lifts a runner at registry 1 to 59 to exactly 60, judged first on a scratch copy by the pinned catalog hash, behind `pre-lift.db`. See `installs.md` for the order, what Recover returns and what is not covered. Fixtures at 28, 45 and 57 come from the real migration history; a drifted catalog is refused with the file and directory untouched.
- **M3, made safe at the takeover point.** `daemon_link::ensure_in` keeps the running recovery daemon (`kept_held`) and starts no new one when a cutover is frozen at a schema this build does not write; the CLI says to reinstall the build that started it. Not done: pausing Sparkle itself, so the app bundle can still change under a runner.
- **M4.** The restore compare, the live re-hash and `quick_check` each have a test that makes the damage (through a test-only seam in `closing.rs`) and a mutant that goes red. `quick_check` does not compare indexes with tables; the test damages an index page structurally, which is what it does catch.
- **M5.** An inert journal is renamed to `<db>-journal.inert-N`, never overwritten.
- **Left, named:** M6 (journal parent not synced, torn entry kept) and M7 (settle wait is about 1.7 s, not 500 ms), both harmless as the review says.
- **New finding, not built.** `startup::inspect` refuses any database below the current registry, including a released one. The first migration after 67 would send every released install back to "pre-cutover schema requires the explicit authenticated cutover". Ordinary admission needs to admit a released database below current and let `Store::open` migrate it.
- **Ran:** store lib 615 passed; the real-crash matrix 6 passed; cli `daemon_link` 11 passed; mutants 47 of 47 red (this round's ten new ones rerun, the earlier 37 not). Clippy clean on touched files. One parallel run of the whole `engine_cutover` filter hit "another process has this database open" once and passed on rerun: the holder scan seeing a sibling test's child between fork and exec.

## Round 4: forward migrations and the lift crash row

- **Forward migrations (`startup::inspect_against`).** A database below the current schema is admitted only if it is `released`, has the authority table, and carries exactly the floor the build that wrote it stamped (`compat::floor_at(version)`). No authority table, `legacy`, `quiesced`, `committed`, or a missing or hand-moved floor still refuse, and a refusal writes no byte. `Store::open` then migrates it and restamps the floor.
- **Where the brief was off.** It says "67 or above", but the authority table arrives with `engine_cutover::migration`, second to last in `MIGRATIONS`, so I keyed the gate on the table, not a number. `legacy` below current stays refused, per "keep refusing anything not released"; only a dev build can have left one.
- **Tests.** A real released database one migration behind goes forward; four other phases/floors stay refused untouched; a generic test passes `current + 1` to show the same rules hold for any later migration. Mutants: legacy-admitted and floor-unchecked both red.
- **Lift crash row.** Six new boundaries (`lift.before-copy`, `after-copy`, `torn-note`, `after-note`, `in-lift`, `after-lift`) killed by SIGKILL at registries 28 and 57. After each: `pre-lift.db` is the exact original from the copy on, the file is the exact original (after rollback) until the lift commits, every seam is held, Recover works, a fresh Prepare goes on to ordinary service, and some kept directory still holds the original. Six mutants, each red on its own boundary: `lift-before-its-copy`, `lift-copy-is-half`, `torn-lift-note-blocks-recover`, `lift-note-after-the-lift`, `lift-commits-before-its-boundary`, `pre-lift-copy-removed-after-lifting`. Full mutant run: 53 of 53 recorded red.
- **Ran:** store lib `engine_cutover` 103 passed; daemon crash matrix 7 passed.

## Surface fixes: R1 and M3

- **R1, fixed on every surface.** The Mac read `cutover_held` as `version-incompatible` and told the owner to update the older side. `ActionCopy.reason` now branches on `what: cutover_held` before the code and says the runner is mid-upgrade and not to update it. `RunnerRefusal.cutoverHeldSentence` (Swift and Kotlin, same words) is the one sentence; the iOS and Android `what:` overloads of `trouble` use it, and the notes at `RunnerRefusal.swift:32` and `RunnerRefusalTests.swift:146` no longer say this code can't reach a phone. Mutant (branch dropped) red on the Mac and in AgentKit. Kotlin ran green; no Kotlin mutant.
- **Phone gap, named.** Only callers that pass `what` get the sentence. A phone call site using the two-argument `trouble(forWord:message:)` still falls to its generic text for this code.
- **M3, now three more places.** `daemon_link::connect` (every command that starts a daemon) refuses to start one when a cutover is frozen at a schema it can't finish. `runner install` asks the runner first and refuses on `cutover_held` before anything is uploaded or restarted. The Mac updater asks `farcooler daemon held` (new, read-only, `{"held":bool}`) and Sparkle's `mayPerform` throws a plain sentence while it's true; `farcooler app update` refuses with `cutover-held`. "Held" is: the attempt's `baseline.json` exists and the authority is past `legacy`. Mutants: connect's check dropped and install's refusal dropped, both red; `UpdateHold.check` dropped, red.
- **Not proved.** The `mayPerform` delegate call itself (a Sparkle session needs a feed; the decision it makes is the tested `UpdateHold.check`). Launchd starting `farcoolerd` directly after an app swap still has no check in `main`. `Baseline` still doesn't record the build. Mac suite floor raised 1949 to 1953 for four new tests; the full Mac suite was not run, only `ActionOutcomeTests`, `UpdateHoldTests`. The screen was unlocked (`CGSSessionScreenIsLocked` absent).
