# ov-1508: the landing check runs the web browser walks

A change under `services/web`, `crates/web` or `crates/client` now runs three
`csp-check.mjs` walks in the gate's `web` job: the fake daemon one, the join
walk, and the two-runner workday. The commands come from `ci.yml`. The path
rule is `web_walks` in `scripts/landing_policy.py`. Other changes skip them.

## Proof

The first two runs used `lane-gate.py --only web` in a scratch clone with
`CARGO_TARGET_DIR` unset. Neither used `--landing`, so they missed the failure
below.

- **Planted failure:** one added commit set an inline `style` attribute when the
  web client started, which the production CSP blocks. The gate failed (exit 1).
  The fake daemon walk failed after 156 s. The workday step failed after 201 s:
  its join walk reached its last step, then reported the CSP violations. Every
  other step in the `web` job passed.
- **Without the plant** (the change's own commit): every step that ran passed,
  including all three walks.
- **Path rule:** `lane-gate.py --self-test` checks that each of the three paths
  plans the walks, and that `Cargo.lock`, `crates/core`, `services/relay` and a
  look-alike path (`services/webx`) do not.

## The landing gate failure, and its fix

The real landing check failed at `services/web/build.sh`: `missing
farcooler_web.wasm, ...`. `--landing` runs each job with `RUNNER_TEMP` set to a
folder of its own (`.../web`). The plan had already filled in
`${{ runner.temp }}` with the parent. So the wasm step wrote to the parent, and
CI's `dist/` step, which reads `$RUNNER_TEMP`, looked in `.../web`.

`at_job_temp` in `scripts/lane-gate.py` now moves the planned paths to the job's
own folder before each step runs. A self-test runs the planned paths through
`run_one_job` with separate folders. It failed before the fix.

After the fix, `lane-gate.py --landing --only web` passed every step that ran,
including all three walks. It used `FARCOOLER_SCRATCH` (99 characters) for the
temp folder and `CARGO_TARGET_DIR`. The planted-failure runs weren't repeated.

## The second gate failure: 12 Rust tests waiting for disk

The next landing check ran all three walks and passed them. It failed on 12
daemon integration tests. Their output gives the reason: `Waiting for disk
space to start this build. 56 GB is free, and builds start at 71 GB`.

ov-1516, already on main, added that hold. The 12 tests run a real debug daemon,
so it read this Mac's free disk, and their builds never started.

`common::as_a_person`, which all 12 go through, now sets
`FARCOOLER_TEST_BUILD_FREE_BYTES`. Only a debug build reads it. With the helper
set to 56 GiB, two of the 12 failed with the gate's messages, at lines 332 and
530. Set to unlimited, all 34 tests in the three files that run those 12 and the
draft approval tests pass.

**How much the walks add to disk.** `target/web-walks` was 4.4 GB of that gate
tree's 32 GB `target/`. The Rust gate's budget in `scripts/gate_disk.py` is
36 GB, so I didn't change it. Without those 4.4 GB, free space would have been
about 60 GB, still under 71 GB, if nothing else changed. I didn't find what
else was using the disk. I also didn't measure a web-only landing against its
8 GB budget, and the walks may exceed it.

## The third gate failure: a button fade, and one leftover folder

**Web.** The workday walk failed with `workday-restart-exited shows 2 filled
buttons`. This branch had changed no web files. ov-1525 traced the message to a
120 ms background fade: New Terminal had lost `primary` but still computed the
accent color. Its commit `b9f6e1337` isn't on main yet, so it's cherry-picked
here. After it, `shell-check`, `work-check`, `board-check`, the fake daemon
walk, the join walk and the two-runner workday passed, run by hand with CI's
commands.

**Rust.** All 7670 tests passed, and `test-leak-check.py` failed the step on one
folder left in TMPDIR, `.tmp3LfIjw`. **I didn't find which test made it, and
nothing here fixes it.** The checker deletes the folder, and the log has only
its name. The three test files that run under `common::as_a_person` (changed by
this branch) passed three times under both leak checks and left nothing. Gate
logs from other trees on Nov 26 and 27 each show one such folder too, but each
of those runs also had a failed test. It can fail a landing again. Listing a
leftover's contents in the checker's report would show where it came from.

## Added time for a web landing

**About 5 minutes warm, about 10 minutes cold.** These are step times from this
Mac, not counting any wait for the heavy-job lock or for load to drop. Warm is
from the `--only web` run without the plant. Cold is from the `--landing` run
after the fix, which started with an empty target directory.

| Step | Warm | Cold |
| --- | ---: | ---: |
| wasm for the bundle (replaces the module-only build) | 30 s | 144 s |
| `services/web/build.sh` into `dist/` | 40 s | 46 s |
| canary `wrangler` dry run, fake daemon build | 1 s | 1 s |
| fake daemon walk | 156 s | 156 s |
| workday step: canary builds, join walk, workday | 70 s | 251 s |
| **Total** | **297 s** | **598 s** |

- The totals include the whole wasm step, although the gate already built the
  module before this change, so they slightly overstate what's added.
- The whole warm `web` job was about 6 minutes. The cold `--landing` run of
  the `web` job alone took 946 s of wall time.
- I didn't time a `--landing` run with the other job families beside `web`.
  They run in parallel, so the added wall time there may be lower.
- A cold lane also uses about 4.5 GB for `target/web-walks`. The walk's canary
  daemon builds there so it can't replace the binaries the Rust tests spawn.

A landing that touches none of the three paths runs no walk, no `dist/` build
and no canary daemon build.

## What the gate needs on the Mac

`wasm-opt` (`brew install binaryen`), Go, tmux, `/usr/sbin/sshd`, Playwright's
Chromium (the gate doesn't install it) and a committed tree. A dirty tree stamps
the `local` channel, and `crates/web/build.sh` refuses to bundle it.

## Still only in CI

- **`sleep-check.mjs`:** the ticket doesn't name it. One run on this Mac ended
  with a 90 s `waitForFunction` timeout. I didn't look into why. It needs its
  own ticket before the gate can run it.
- **Changes outside the three paths:** `crates/daemon`, `crates/core`,
  `crates/cli`, `proto` or `Cargo.lock` can also break the workday walk. The
  ticket limits the walks to three paths, so CI is what catches those.
- **The size gate on `dist/`:** the Mac's `wasm-opt` isn't CI's pinned build.

## `ci.yml` changes, not yet run in CI

- Installing tmux is now its own step. Inside the walk step, its `apt-get` made
  the gate skip the whole step as runner setup.
- Both `--fake-daemon` paths now follow `CARGO_TARGET_DIR`.
- The canary dry run writes under `$RUNNER_TEMP` instead of `/tmp`, because the
  gate now runs it too.

`scripts/actionlint.sh` passes.
