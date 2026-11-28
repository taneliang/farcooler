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
