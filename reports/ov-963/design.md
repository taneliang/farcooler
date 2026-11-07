# ov-963 design: Commit on a real install

Written before code. It covers three things: Commit with a closed backup, the boundary matrix, and old-build stdio after Recover.

## Corrections to the brief

Each was read against this tree (894fa07af).

- **Registry.** `preflight` admits 60 and 65 to 67 (`engine_cutover.rs:208`). A source at 61 to 64 is refused, so an install there stays in recovery with only Recover. Real installs were not read. This is named for the owner-copy step, and is not changed here.
- **R1 and R2 are closed.** `recovery_seat_in` now fills the pane terminals, and `hold` refuses a non-legacy database. Both landed in 09a8aae02.
- **Commit is not simply missing.** Three more gaps would strand an install even with Commit added:
  - `schema:unknown` commits, then Release refuses forever and Recover refuses too. Commit must refuse first.
  - Verify runs only after the commit, so a failure there has no exit. Commit must prove it inside the transaction.
  - After Finish or Recover the recovery daemon keeps serving, so nothing returns the runner to ordinary service (R4).

## Commit

### Shape

`Cutover::Commit` is one new op in the closed vocabulary, served only in recovery, with the same caller check as the other recovery ops. The owner runs:

1. `prepare` on the live daemon, which drains and stops it.
2. `prepare` in recovery, which captures, upgrades and freezes.
3. `commit`, which backs up, proves and commits.
4. `verify`, `release`, `finish`.

This keeps the steps the contract and CLI already have. No product shape changes: Commit is the missing word, not a new flow.

### Who holds the closed backup

`transfer::commit(db, owners)` holds the kernel owner proof for the whole operation, from before the copy to the commit. The backup and the permit are never apart from that lock.

The files live inside the attempt's own artifact directory, so ordinary startup stays refused while they exist and Finish carries them to `-resolved-<attempt>`. Nothing deletes them.

```
farcooler.db.cutover/
  baseline.db  baseline.json  intent.json     original source, from Prepare
  closed-<n>/db                               the closed quiesced database
  closed-<n>/SNAPSHOT.json                    its hash, length, mode, manifest, attempt
  journal/backed_up-<n>.json                  outside the replaceable database
  journal/committed.json  verified.json  released.json  resolved.json
```

`<n>` is the first unused number. A permit lives only in memory, so a crash can never reuse one. A later Commit takes a new number and the crashed copy stays as evidence. Files are create-new, and each is synced with its directory.

The early baseline does not count as this backup. The contract keeps them separate: the baseline restores the original source, and the closed backup proves what is about to be mapped.

Only the database is backed up. It is the only thing Commit changes. `Preflight` never follows a stored path into owner resources, and `resources` in the baseline are identities and generations, not files.

### Order, all under one proof

1. Validate the artifacts. The proof's attempt must be the baseline's.
2. If the database already says `committed` or `released` for this attempt, add any missing journal entry and return `resumed`. This is the exit after a crash between commit and journal. Database truth wins over a stale journal.
3. Require `quiesced`, the frozen attempt and the same protected resources. Refuse `schema:unknown` here, while Recover still works.
4. Close the Store. Refuse any sidecar, take `BEGIN EXCLUSIVE`, run `quick_check`, close.
5. Copy to `closed-<n>/db`, and write the snapshot.
6. Restore independently: copy from the backup to a scratch path and compare the hash, typed manifest and resources. Delete only that scratch copy.
7. Re-hash the live file. It must still equal the snapshot.
8. Journal `backed_up-<n>`, then mint the permit.
9. Reopen and run `engine_commit`. Its last step inside the transaction is the full `verify::check`, so a commit that could not verify rolls back and stays frozen.
10. Journal `committed`.

`CommitPermit` keeps its private fields. It gains a second kind of evidence: `Kernel { owners_hash, protected_resources }` beside the rehearsal's `Quiescence`. Production never builds synthetic writer strings, and `PrivateRehearsal` stays synthetic-only. The new constructor is private to the store crate, and it takes `&JoinedOwners`.

### After it

- Verify and Release also require the closed backup that the attempt row names, with its bytes intact. A lost backup refuses without repair.
- Verify, Release and Finish journal their stage after database truth. Finish adds any entry a crash left missing.
- A successful Finish or Recover ends the recovery daemon with exit 75 after the reply, so launchd and systemd restart it into ordinary service. Their restart rule is on-failure. Without a supervisor, the next client starts it.

### Restoring after Commit is not built

Committed state recovers forward only. The contract's explicit restore, with a new incarnation and external-effect reconciliation, stays unbuilt. The closed backup and baseline are kept for it. Until it exists, a failed Verify after Commit leaves the runner in recovery with its data intact. The check inside the transaction makes this need a change to the file after Commit.

## Old-build stdio (R3)

An old process holds a descriptor on the inode that Recover replaces. Its later writes go to the unlinked file.

**Decision: refuse, at the proof.** `JoinedOwners::acquire` also asks the kernel which other processes have the database file open, and refuses with their pids. macOS uses `proc_listpidspath`. Linux uses `/proc/*/fd` by device and inode. A failed scan refuses. Nothing is signaled. `inspect` reports the pids, so the owner can see who to close.

This covers every stage, the rehearsal's included. It also meets the contract's rule that an unjoined descriptor blocks the transfer.

Why this leaves no window:

- The scan runs before the bytes move.
- An fd opened after the scan and before the rename reads the frozen file, whose floor refuses an old build (`old-reader-result.json`).
- An fd opened after the rename gets the new inode, so its writes are kept.

Restoring in place through the SQLite backup API was rejected. It would keep the inode but changes header bytes, so "exact original bytes" would have to weaken to a manifest match. The scan keeps the stronger oracle.

The scan cannot see another user's process. The database is private to the runner's one Unix user.

## Telling the stdio device (M6)

A new `DomainError::CutoverHeld`, with a sentence of its own and the same wire treatment as `NewerData`, is returned by `cutover_gate::admit` and the Store gate. `stays_up_for` includes it, so standalone stdio sends the refusal to the device.

## Boundary matrix

### Interruption

`engine_cutover::boundary::reach(name)` is empty in production: no environment read compiles in. Under `cfg(test)` or the `testing` feature it sends the process SIGKILL if `FARCOOLER_TEST_CUTOVER_KILL_AT` names that boundary. `ALL` lists every name.

The helper is the integration test binary re-executing itself: a real process running the real `cutover_admin` stage, killed by the kernel with real hot journals and torn files.

SIGKILL cannot prove `fsync`, since the page cache survives. Sync order is shown by source reading and the artifact tables, and is not claimed as crash-proven.

### Boundaries

Before and after each durable step: hold, baseline database, baseline file (with a torn write), intent (with a torn write), upgrade (inside and after the transaction), closed copy, snapshot, independent restore, permit journal, mapping commit (inside and after), committed journal, verify, release (inside and after), finish, and Recover's rename and retire.

### Fixtures

The raw schema 60 fixture moves from the test module to `testing.rs`, so both crates use one. A current-schema legacy runner covers the hold path.

### Oracle at each boundary

1. The helper died by signal 9. A boundary that is never reached fails, so a renamed step cannot pass silently.
2. The artifact files and the database phase, schema, floor and frozen attempt equal that boundary's expected row.
3. Reopening through real `farcoolerd` main, `--stream`, standalone `--stdio`, `Service::open_in` and `Store::open`:
   - Before admission: main serves recovery only, stream exits nonzero, stdio tells the device, and the in-process seams refuse.
   - The database and its sidecars are byte-identical afterward. The home tree gains no `install-id`, registry or tmux server.
4. From the crash, with a fresh proof:
   - Forward reaches `released`, then ordinary service.
   - Before commit, Recover reaches the exact original bytes, then ordinary service.
   - After commit, Recover is refused and changes nothing.

### Each mutant must go red

`reports/ov-963/mutants.py` applies one defect per boundary, requires the build to succeed and a named test to fail, then restores the file byte-exact. Setup failure never counts as red. Mutants are listed there with their results.

### End to end through the socket

A real main, and a caller that really is a person to `recovery_seat`: an orphan on a fresh pty, outside every pane. It runs Prepare to Finish over the real socket. Main exits 75, and the next main serves ordinary RPC. A refused caller in a pane proves nothing here, so the test checks that the walk returned `Person` by requiring success.

### Old build

A child process holds a raw SQLite connection, as a pre-lock build does. Recover must refuse and name its pid. Once it exits, Recover succeeds. The mutant without the scan shows the child's write vanishing.

## For the ov-936 rehearsal

To be restated with results in `reports/ov-963/result.md`:

- `JoinedOwners::acquire` now refuses on another process's descriptor, so the rehearsal fails loudly if anything holds its file.
- `engine_commit` verifies inside its transaction.
- `CommitPermit` has two evidence kinds. The rehearsal's path is unchanged.
- The raw60 fixture moved.
