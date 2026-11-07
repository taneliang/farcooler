# ov-940 phase 2 status (private implementation; not a release claim)

Current at the fix round for the review of e2ac7e783. Blockers B1-B6 and M1 are fixed. The earlier text in this file described a join proof and live Prepare that no longer exist.

## How ownership works now

* Every ordinary `Store::open` holds a shared `flock` on `<db>.owners`, and runs the gate under it. The kernel drops it on exit or crash.
* `JoinedOwners::acquire` takes that lock exclusively without blocking and is the only issuer. The proof owns the lock, has no Clone or serde, and each stage consumes it. A holder blocks it; nothing is signaled.
* A transfer runs only in a recovery process. A live daemon's Prepare drains its writers, writes `hold.json` and stops. The next start refuses ordinary startup and serves recovery, where Prepare runs. A database below the current schema goes straight to recovery.
* `transfer::begin` captures, publishes the baseline and intent, and upgrades inside one `BEGIN IMMEDIATE` transaction on one connection. Its read descriptor outlives the commit, because closing one inside the transaction would cancel SQLite's POSIX locks.
* `Recover` ends any uncommitted attempt. Not frozen: artifacts are retired. Frozen: the database must equal a replay of the upgrade on the baseline, then the original bytes are restored atomically. A later write refuses and keeps everything. Committed or released refuses. Evidence is renamed, never deleted.
* Recovery has no Store for a caller's panes, so `caller_seat::recovery_seat` walks the process table with tmux servers as chain ends. Pane and agent callers are refused; the owner uses a plain terminal or the Mac app. The caller's process start is rechecked per request.
* Registry 60 is pinned by `SOURCE60_SCHEMA_HASH`, checked before any table is read. Verify requires growth into exactly the pinned later catalog with every added table empty.
* The baseline binds root, file name and identity, registry, floor, catalog, raw and DB hashes, owner receipt, resources and generations. `frozen:<attempt>` commits with the upgrade.

## Evidence (HEAD at the commit carrying this file)

* Store 562 passed, 0 failed, 3 ignored. Protocol 72. CLI workflow 6. Daemon `recovery::` 7; wider affected filter 271 passed, 1 failed (below). Clippy on all four crates clean. File-size, migration and doc-comment checks ok.
* `fix-controls.py`: 21 mutants, each putting one reviewed defect back, all red; results in `fix-controls-results.json`. These are mutants of the finished code, not a chronological red-first history: the tests and the fix landed together because the API changed.
* `old-reader-result.json`: the real registry-60 Store at d625aaeb opens a raw schema60 fixture and is refused `NewerData` on the file `begin` leaves.

## Open, not claimed

* **Release blocker:** Commit over RPC is parent-owned, so an existing install reaches recovery and Prepare but cannot return to ordinary service on this build. Recover plus the previous build is the only way out. Verify, Release and Finish have no production path to `committed`.
* Fences and fresh phase are untouched pending ov-945; fresh installs stay legacy.
* A process from a build before this lock is invisible to it. Its write is refused during capture and caught afterward by Recover and Verify, not prevented.
* The manual-delivery test sets `released` through `testing::released_transfer_for_tests`, which uses the unit proof: that fixture's background tasks outlive its Service and keep the lock. It tests what follows a cutover, not who may start one.
* Not built: the verification.md boundary matrix through actual main, stream and stdio, and the owned helper interrupted at each durable boundary. Crash states here are constructed.
* Review minors M2, M4, M5 and M6 are unchanged. M3 no longer strands anything: a failed drain still stops the daemon, which restarts ordinary. M7 is fixed for the tests named, apart from that boundary matrix.
* `people::actor_tests::every_write_that_names_an_actor_is_listed` fails on `WorkflowCommand`. Its inputs (the proto, the test, people.rs) have no diff since 7f6edddd2, so it predates this branch by inference; I did not build the base to confirm.
* `scripts/copy-lint.py` reports 5 Android title-case strings. There is no diff under apps or android in this range.
