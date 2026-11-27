# ov-1516 Build scratch within a disk budget

All four acceptance lines are done and tested. For acceptance 3, the disk now comes back in two ways:

- **Dev and test builds use line tables only.** Measured back to back, a cold full-workspace build dropped from 26.1 GiB and 496 s to 17.5 GiB and 374 s.
- **Builders start from a clone of landed main's third-party build.** That halves a typical builder's cold build time and saves another 1.8 GiB of disk.

See [Shared dependencies](#shared-dependencies-acceptance-3).

**One step is left for the orchestrator.** The engine seeds only when the adapter config names a `dependency_seed` command, and an agent can't publish config. The config to add is under [Turning it on](#turning-it-on).

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

## Shared dependencies (acceptance 3)

### How it works

The review ruled out two sources for a seed. Builder scratch can't be trusted, because Codex can write anywhere in `/tmp`. The gate tree is built with `debug=0`, so its units never match a builder's. The seed therefore comes from a third place: the engine builds it itself, with the builders' own flags, in a folder only the engine writes to.

- **Config.** `CodingConfig.dependency_seed` is an optional command. It's left out of the pinned bytes when absent, so existing pins don't move. It's validated like any other configured command and shown in the workflow summary.
- **When it runs.** It runs when a landing push lands (`PushVerdict::Landed`). It also runs when a build starts and no seed exists yet, which covers first use. Only one seed builds at a time. A commit whose seed failed isn't tried again; the next landing is. It pauses for a landing check as builders do (`owned_yielding` with the builder yield).
- **When it waits for disk.** A seed builds the whole workspace cold and keeps everything it wrote while it pauses for a landing check. So it waits until there's room for the landing check's 61 GB beside a full seed: **91 GB** free, which is 61 GB, the seed's 20 GB peak (`SEED_PEAK`, measured at 17.5 GiB) and the 10 GB margin. A builder waits for only 71 GB, which isn't enough room for a seed.
- **Where it runs.** It runs in a detached checkout of the landed commit at `<engine root>/dependency-seed/<sha>.checkout`. `FARCOOLER_SEED` points to `<sha>.partial`, an empty `0700` folder, and the environment is exactly what the config names. On success, the partial folder becomes `<sha>`, and the ledger's `current` moves to it.
- **What builders get.** Each Build or Review step's new `$FARCOOLER_SCRATCH` gets an APFS `clonefile` of every top-level entry of the current seed. This happens before the agent starts, under the same lock that a seed swap takes, so a seed is never removed mid-clone. Clones share blocks until written, and a builder's writes never reach the seed (tested). Without a seed, or off APFS, the build simply starts cold.
- **Deletion.** `dependency-seed/ledger.json` names every folder before it's made. Removal runs on a blocking thread without the seed lock, because a failed seed's partial output can run to many gigabytes. No lock is needed: builders clone only `current`, and a folder is retired only after `current` has moved off it under the lock. The engine deletes only folders the ledger names, by exact path, inside the seed folder, and never while `lsof` shows a file open in them. The previous seed goes once the new one is ready, and a failed seed's partial output goes straight away.

The cargo-specific half is in the repository: `scripts/dependency-seed.py`.

- It runs `cargo test --workspace --no-run` and `cargo build --workspace --bins` with no `CARGO_*`, `RUSTFLAGS` or `RUSTC_*` variables. Builders have none either; I checked this lane's own environment. Profile, Rust version and lockfile therefore all match. The profile comes from the landed commit's `Cargo.toml`, so it matches the builders' after a profile change too.
- It then keeps only third-party units. It reads the hash of every `.fingerprint/<package>-<hash>` whose package is a workspace member (`cargo metadata --no-deps`). It then removes every `deps`, `build` and `.fingerprint` entry with one of those hashes, any entry not named for a unit, and everything else under `debug/`, including `incremental/`. No workspace unit is seeded, so a seed can't hide a builder's changed source behind file modification times.
- `--self-test` covers the strip. CI runs it. With either half of the filter broken, it fails.

### Measurements

All runs were on this machine, Nov 28, from a clean `git archive` of this branch inside this lane's scratch, with Rust 1.98.1, default jobs and no `CARGO_*` variables. "New disk" is `du -sk` of the target folder afterwards.

**Line tables only (Cargo.toml `[profile.dev] debug = "line-tables-only"`).** The pairs ran back to back:

| Cold build | Full debug info (before) | Line tables only (after) |
| --- | ---: | ---: |
| Whole workspace: `cargo test --workspace --no-run`, then `cargo build --workspace --bins` (what a seed builds, and close to a builder that tests everything) | **26.1 GiB**, 496 s, 2,203 s CPU, load 19 to 26 | **17.5 GiB**, 374 s, 1,517 s CPU, load 8 to 21 |
| of which `deps/` | 18.0 GiB | 10.2 GiB |
| of which `incremental/` | 7.1 GiB | 6.5 GiB |
| Typical builder: `cargo test -p farcooler-daemon -p farcooler-store -p farcooler-workflow --lib --no-run` | 5.5 GiB, 153 s, load 3 to 8 | 4.1 GiB, 121 s, load 8 |

The earlier round measured the same whole-workspace build at 38.3 GB (811 s, load 30 to 43), with a different counter and under heavier load. The back-to-back pair above is the comparison to use. Backtraces keep files and lines. A debugger still steps by line but can't show variables; `CARGO_PROFILE_DEV_DEBUG=true` restores full debug info for one build. CI and the landing gate set `debug=0`, so they don't change.

What's left is `incremental/`, at 6.5 GiB. Builders rebuild the same crates many times and incremental data speeds those rebuilds, so I kept it on.

**Seed, earlier round (full debug info).** The seed is **1.9 GB** after the strip: it kept 3,149 entries and removed 221 workspace units. A typical builder command, cold and then from the seed, ran back to back at load 16:

| | Cold | From the seed |
| --- | ---: | ---: |
| Wall time | 250 s | **127 s** |
| CPU (user) | 771 s | 309 s |
| Crates compiled | 327 (17 workspace) | 44 (17 workspace) |
| New disk in scratch | 6.89 GiB | 5.99 GiB (+1.78 GiB shared with the seed) |

Together, a cold whole-workspace builder needs about 17.5 GiB instead of 26 to 38 GB. A seeded builder needs about 1.8 GiB less again, and builds in half the time.

- **Time halves with the seed.** The 283 third-party crates the seed supplies are built at `opt-level = 2` (`[profile.dev.package."*"]`), which makes them the costly part of a cold build. A relocated target works: cargo rewrites build-script output paths from the seed's folder to the builder's.
- **27 third-party crates still rebuild**, including `syn`, `serde_derive` and `url`. A `-p` subset selects other features than `--workspace` does, so those units have different hashes. They're a sixth of the crates and cheap ones.

### Turning it on

Add this to the workspace's adapter config, then publish it:

```json
"dependency_seed": {
  "argv": ["/usr/bin/python3", "scripts/dependency-seed.py"],
  "environment": {"HOME": "/Users/e-liang", "PATH": "/Users/e-liang/.cargo/bin:/usr/bin:/bin"},
  "timeout_ms": 5400000
}
```

The arguments resolve inside the seed checkout. The first seed is made by the next landing, or by the next build if no landing comes first.

## Verification

**Third round (profile, seed disk threshold, removal off the async workers):**

- `cargo test -p farcooler-daemon --lib -- dependency_seed builder_resources owned_directories`: 21 passed, 1 ignored (the live Codex probe).
  - `a_seed_waits_until_a_landing_check_keeps_its_need_beside_a_full_seed` replaces `a_seed_waits_for_the_disk_a_build_needs`. A seed holds at 71 GB free (room for a builder) and at one byte under 91 GB, then builds at 91 GB. With the seed back on the builder threshold, this test fails.
- `cargo clippy -p farcooler-daemon --tests`: clean. `doc-comment-check.py`, `daemon-layers.py`, `copy-lint.py`, `file-size-budget.py` and `dependency-seed.py --self-test` all pass.
- The measurement builds above ran in this lane's scratch, and I deleted each target folder afterwards.

**Second round (seed):**

- `cargo test -p farcooler-daemon --lib -- dependency_seed`: 4 passed.
  - `a_landing_seeds_the_next_builds_scratch_and_retires_the_old_seed`: a ticket lands through a group. The landing starts a seed of the landed commit. The old seed goes. The next ticket's build finds the landed commit's unit in `$FARCOOLER_SCRATCH`, and its write doesn't reach the seed.
  - `a_failed_seed_leaves_builders_cold_and_is_not_rebuilt_for_the_same_commit`
  - `only_folders_the_ledger_names_are_removed_and_never_while_open`: an unrecorded folder and a ledger entry outside the seed folder are left alone, and an open file keeps a retired folder.
- Mutation checks:
  - With the landing refresh removed, the landing test fails.
  - With the scratch clone removed, the landing test fails.
  - With either half of the script's filter broken, `--self-test` fails.
- `cargo test -p farcooler-daemon --lib` over `workflow_runtime::`, `workflow_summary`, `workflow_drafts` and `workflow_starter`: 642 passed and 2 failed.
  - `the_native_launch_refuses_the_quarantined_binary_behind_an_npm_codex_launcher` is the `node` architecture failure described below.
  - `ov1425_publications_carry_compatible_runs_and_restart_other_runs` failed under the full run at load 16 to 28, then passed 3 out of 3 alone.
- Other checks, all clean:
  - `farcooler-store` (660 passed) and `farcooler-protocol` (73 passed); the CLI's `landing_templates` tests pass.
  - `cargo clippy` on the four touched crates with `--tests`.
  - `daemon-layers.py`, `doc-comment-check.py`, `copy-lint.py` and `file-size-budget.py`.

**Earlier round:**


All runs used `CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"`.

- `cargo test -p farcooler-workflow -p farcooler-store`: all passed (660 in the store library).
- `cargo test -p farcooler-daemon --lib -- workflow_runtime::`: 528 passed and 83 failed on the first run, because I had built only the library and the native tests need `farcoolerd` and `farcooler` built beside it. After `cargo build -p farcooler-daemon -p farcooler-cli --bins`, 82 of the 83 passed. The last one, `the_native_launch_refuses_the_quarantined_binary_behind_an_npm_codex_launcher`, can't run the `node` on this machine's PATH ("Bad CPU type in executable", os error 86: an x86 build with no Rosetta). It fails before any engine code runs.
- That run made no new folder in `/private/tmp`.
- The two mutation checks above failed as they should, and I restored the files afterwards.
