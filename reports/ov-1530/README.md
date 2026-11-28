# ov-1530: the landing check runs the sleep check

## The 90 s timeout

**What ov-1508 saw was the stale wait that ov-1523 fixed.** ov-1508's final
branch contained ov-1490 (`8c996be15`), which changed a new browser's first
screen. The sleep check still waited for the old one, `#enroll`.

To confirm it, I ran the script from before ov-1523's fix (`77af70117^`) against
a current build. It ended with `chromium could not run: page.waitForFunction:
Timeout 90000ms exceeded.` after 90.89 s, before any sleep case ran.

**Why 90:** I found no reason. The number has been there since the script's
first commit (`337eef904`, ov-840), and neither the script nor the commit
message explains it. `csp-check.mjs` used 120 s for the same waits then. The 90 s
isn't the run time or a sleep. It's only how long a wait for a screen lasts
before it fails, and a passing run doesn't spend it.

**Now 30 s**, in one documented constant, `SCREEN_WITHIN_MS`. It covers the five
places that used 90 s (one is the `statusSays` default). No case, assertion or
other limit changed.

## Measured times

On this M-series Mac, with CI's command (`--engines chromium --split`):

| What | Time |
| --- | --- |
| Slowest of 63 waits that had the 90 s limit (3 runs, load 6 to 8) | 785 ms |
| Whole run, the same 3 runs | 134 to 138 s |
| Landing check step, planted run | 135 s |
| Landing check step, final run (with the 30 s limit) | 136 s |
| CI's step on Linux, 6 green `main` runs (46121521584 to 46140168285) | 139 to 143 s |

The three timed runs used a temporary copy of the script that logged each wait.
Their build had the planted failure below, so they failed the three dead-link
cases and nothing else. The plant only removes a log line. I didn't time CI's
waits one by one, or the waits on a heavily loaded machine.

## The landing check

A change under `services/web`, `crates/web` or `crates/client` now runs
`sleep-check.mjs` in the gate's `web` job, using the command from `ci.yml`. It
follows the same path rule as the `csp-check` walks. `lane-gate.py --self-test`
checks that those paths plan it, that other paths don't, and that no
`sleep-check` or `csp-check` step is left to CI for a web change.

**It adds about 2 min 15 s to a web landing.** It reuses the walks' `dist/` and
fake daemon, so it needs no extra build.

### Planted failure

In a scratch clone, one added commit made `logLine` in
`services/web/src/client/app.ts` log nothing, so a dropped link no longer said
why. `lane-gate.py --only web` failed:

- `sleep-check.mjs`: FAIL (exit 1) after 135 s. `bg-1min-dead`,
  `bg-10min-dead` and `lid-10min` each reported `no line saying why the link
  ended`.
- `npm test`, the fake daemon `csp-check` and the join and workday step passed.
  Of the steps that failed, only the sleep check was caused by the plant.

The other failure in that run was the Go step, explained below. It isn't from
the plant.

### Without the plant

At `708b87445`, `lane-gate.py --only wire,doc-comments,web` passed every step
that ran. All 8 sleep cases passed. The three dead-link cases showed the notice
after 14.14 to 14.16 s, within the 17 s limit. It's a partial gate: `relay`,
`site`, `tunnel`, `rust` and the Swift jobs didn't run. This change only edits
the gate's two Python files, the check and this report.

## Found on the way: the Go wasm step depends on the shell's environment

In the planted run, `GOOS=js GOARCH=wasm go test` failed with `total length of
command line and environment variables exceeds limit`. Go's `wasm_exec.js` caps
their combined size. This shell has a 2.6 KB `FASTLANE_SESSION` variable. The
final run unset it (`env -u FASTLANE_SESSION`), and the step passed.

I didn't change this, and I didn't check whether the engine's gate environment
carries that variable. If it does, this step can fail a web landing. It's worth
its own ticket.
