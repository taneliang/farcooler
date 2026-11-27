# ov-1516 Build scratch within a disk budget

Three of the four acceptance lines are done and tested. The shared-dependency line is **not done**. The safe sources for shared compiled dependencies don't match builders' build flags, and the sources that do match are writable by sandboxed agents. Details and a proposed next step are under [Shared dependencies](#shared-dependencies-not-done).

## What was on this machine (Nov 28, 01:40)

`/private/tmp` held **6,570** `farcooler-scratch-wf1-*` folders:

| | Folders | Size |
| --- | ---: | ---: |
| Empty | 6,568 | 0 |
| ov-1435, recorded Nov 26 22:17, run cancelled, hold never reported | 1 | 22 GB |
| ov-1517, a live builder | 1 | 2.1 GB |

By creation date: 61 on Nov 25, 2,770 on Nov 26, 3,440 on Nov 27 and 299 by 01:40 on Nov 28.

The canary database records **843** scratch folders, and only 5 were still pending: ov-1435 and four live builders. None of the 6,568 empty folders is a recorded path. The engine therefore never made them in production. Scratch is named by a digest of the engine root and the effect. Until this change, every test that ran a Build or Review step made its scratch in `/tmp`, under its throwaway engine root, where no recorded path could ever name it. That fits the creation rate. I didn't replay the old code to prove it.

## Scratch is released when its run is over (acceptance 1)

Before this change, a scratch folder was released only when the step that made it reached `settled` in the outbox. ov-1435 shows the gap. Cancelling a Build whose native hold never reported leaves the effect `uncertain` and the item `Uncertain` with a `cancel:` run. That state never settles, so the 22 GB stayed.

`owner_ended` (`crates/daemon/src/workflow_runtime/owned_directories.rs`) now treats the owner as finished when it is:

- `settled` or `suppressed` (never ran), as before; or
- `uncertain`, and its run is over: Done, Failed or Cancelled, archived, cancelling (`cancel:` run), or the item has moved on to another effect; or
- `uncertain` and recorded **a day** ago or more (acceptance 2).

The store now returns the owner's dispatch state (`workflow_cleanup_owner_state`) instead of a yes/no for "settled". A quarantined resource still returns nothing. Deletion keeps every earlier safeguard:

- the exact recorded path, device and inode;
- an atomic `rmdir` when the folder is empty;
- an `lsof` check before a non-empty folder goes, with a retry while any process has a file open in it.

Tests (`owned_directories/tests.rs`):

- `a_cancelled_build_releases_its_scratch_though_its_hold_never_reports`: the ov-1435 shape. The build is admitted, the runner restarts and the item goes Uncertain, then it is cancelled. The stop can't prove the build ended, and the next cleanup round deletes the 1 MB of scratch.
- `an_uncertain_build_is_an_orphan_a_day_after_its_scratch_was_recorded`: kept 1 ms short of a day, deleted at a day.
- `an_open_file_keeps_an_ended_runs_scratch_until_it_closes`: an open file keeps the folder and records a retry. Closing it lets the folder go.

With the uncertain branch disabled, all three fail and the existing release test still passes.

**Dry run on this machine.** I read the canary database read-only and applied the new rule:

| Item | Owner | Item phase and run | Age | Result |
| --- | --- | --- | ---: | --- |
| ov-1435 | inactive, `uncertain` | Uncertain, `cancel:` | 27.7 h | released (22 GB) |
| ov-1490, ov-1491, ov-1516, ov-1517 | active, `admitted` | Running | under 2 h | kept |

The other pending rows are landing-gate log folders inside their 7-day retention. This rule doesn't change them.

## Orphans at runner start and hourly (acceptance 2)

No new sweep was needed. The cleanup round runs these recorded releases at runner start (`recover_trees`), after work finishes, and hourly in service mode (`CLEANUP_MAINTENANCE_INTERVAL`, which also drives process-log pruning). A pending scratch record that isn't ready yet is looked at again on the next round. The day limit means an uncertain owner's folder goes within an hour of turning a day old.

**Counts.** These are the numbers before and after on this machine:

| | Before | After |
| --- | ---: | ---: |
| Recorded scratch pending release | 5 | 4 at the next round once the runner runs this build (ov-1435's 22 GB goes) |
| New `/private/tmp` scratch folders from the `workflow_runtime` test module | 6,568 empty folders built up over 3 days | 0 (I listed `/private/tmp` before and after the run) |
| Unrecorded empty folders in `/private/tmp` | 6,568 | 6,568: the engine deletes only paths it recorded |

The 6,568 empty folders need the owner. The engine never recorded them, and the constraints rule out a deletion by pattern, so I didn't remove them. This command removes only empty folders older than a day; `rmdir` refuses any folder with something in it:

```sh
find /private/tmp -maxdepth 1 -type d -name 'farcooler-scratch-wf1-*' -empty -mtime +1 -exec rmdir {} +
```

Tests now make scratch beside their own engine root, inside the test's temporary directory (`Exec::scratch_base`, test builds only). The ignored live Codex probe pins `/tmp` again, because Codex's sandbox can write only there.

## New builds wait for disk (acceptance 4)

The old rule failed a build outright when either the engine volume or the worktree had under 25 GB free. The run then needed a person to retry it. Now a Build step that would start with under **71 GB** free on the fuller of the two volumes returns `ResourceHold`. 71 GB is the landing check's 61 GB (`scripts/gate_disk.py`: 36 GB Rust gate budget plus 25 GB daemon reserve) and a 10 GB margin for builds already running. The hold:

- launches nothing and spends no attempt; the reducer now accepts `ResourceHold` from a Build step as well as a group's Run;
- puts the reason on the item, which the board projects as WaitingSlot with the reason as the ticket's summary: "Waiting for disk space to start this build. 70 GB is free, and builds start at 71 GB so a landing check keeps the 61 GB it needs. It starts by itself once space is free.";
- is offered again at most once a minute, including after a restart, like a gate's hold; the next answer clears the reason.

Tests (`builder_resources/tests.rs`):

- `a_build_waits_on_its_ticket_while_disk_is_short_and_starts_once_there_is_room` covers the hold, a runner restart, the `workflow get` view and the start once space frees. With Build holds refused in the reducer, it fails.
- `a_build_needs_the_landing_checks_space_and_a_margin` checks the two thresholds.
- `a_low_disk_hold_never_touches_adopted_output` replaces the old refusal test.

## Shared dependencies (not done)

**Before.** ov-1435's 22 GB target held 15 GB of `deps` (9.7 GB third-party crates, 5.5 GB workspace crates and test binaries), 6.8 GB of `incremental` (workspace only) and 0.4 GB of examples. Third-party crates are about 45% of a cold builder target. For timing, this lane's own cold build of the `farcooler-daemon`, `-store` and `-workflow` lib test binaries took 5 min 20 s and 5.4 GB, with 4 jobs and load average 16 to 45.

**Why I stopped.** Sharing needs a source of compiled dependencies that builders can trust and that matches their flags. Neither source available today has both:

1. **Builder scratch is not trustworthy.** Codex's `workspace-write` sandbox can write anywhere under `/tmp`, so a Codex agent can write into any scratch folder, not just its own. A seed copied from scratch could carry a planted proc-macro `.dylib` or build-script binary. Cargo would treat it as fresh and load it into `rustc` in the next unsandboxed builder. That is a sandbox escape. `sccache` isn't installed, and its local cache would have to sit outside `/tmp` for the same reason.
2. **The landing tree is trustworthy but built differently.** The gate's group tree is engine-private and builds landed code. But `lane-gate.py` uses the CI job's environment, which sets `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0`. Cargo hashes the profile into each unit's metadata, so builders using the default profile would never reuse those artifacts.
3. **Workspace artifacts must never be seeded.** Cargo decides freshness for path packages by file modification time. A seed newer than a builder's checkout would let an old artifact stand in for changed source. A seed must hold third-party units only.

**Proposed follow-up.** Once the owner chooses the flags, file it as "Engine: builders reuse landed third-party builds":

- Builders get the gate's profile environment. That needs a decision: line numbers leave builder backtraces, but deps also shrink a lot.
- After a group lands, the engine clones the gate tree's third-party units into an engine-owned seed with APFS `clonefile` and removes workspace units by package name from `cargo metadata --no-deps`.
- Each builder's `$FARCOOLER_SCRATCH/target` starts as a clone of that seed. A clone shares disk blocks until written, so a cold build would cost roughly the 12 GB of workspace output instead of 22 GB, and skip compiling about 10 GB of third-party crates.

## Verification

All runs used `CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"`.

- `cargo test -p farcooler-workflow -p farcooler-store`: all passed (660 in the store library).
- `cargo test -p farcooler-daemon --lib -- workflow_runtime::`: 528 passed and 83 failed on the first run, because I had built only the library and the native tests need `farcoolerd` and `farcooler` built beside it. After `cargo build -p farcooler-daemon -p farcooler-cli --bins`, 82 of the 83 passed. The last one, `the_native_launch_refuses_the_quarantined_binary_behind_an_npm_codex_launcher`, can't run the `node` on this machine's PATH ("Bad CPU type in executable", os error 86: an x86 build with no Rosetta). It fails before any engine code runs.
- That run made no new folder in `/private/tmp`.
- The two mutation checks above failed as they should, and I restored the files afterwards.
