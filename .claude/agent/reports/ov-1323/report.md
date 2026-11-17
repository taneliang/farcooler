# ov-1323 — the real CLI Claude offer test

## Investigation

The test sends input immediately after `terminal create`, before proving that
its live pane's interactive shell can read it. Creation confirms the tmux
window's tags and life, not shell initialization. `terminal send` independently
opens a live runtime inventory, resolves the terminal there, and sends literal
keys. Its success acknowledges tmux's write, not execution by the shell.
Polling `runningAgent` afterward cannot recover a command lost during startup.

Evidence from the two Nov 17 gate logs in `/Users/e-liang/Dev/overnight`:

- `.claude/agent/reports/ov-1308/gate-swift-tests.log:6999`: `terminal send:
  the CLI exited 1; stderr: error: resource not found`; the test ended after
  4.590 seconds (line 7000). This failed at the live-runtime resolution/send
  step, before the detection loop, despite a successful creation response.
- `.claude/agent/reports/ov-1320/gate-2-swift-tests.log:7354-7360`: send
  succeeded, but the last successful list still said `runningAgent: ""` after
  the entire polling loop; both `runningAgent == "claude"` and `offers` failed.
  Lines 7590-7594 record peak load `[34.3223, 20.3574, 12.9644]`. This was not
  the old bug of stopping at the first empty string: the loop already waited
  specifically for `"claude"`.

Neither gate log records the shell screen at send time, so they alone cannot
prove exactly which startup step lost the race. The shared missing precondition
is visible in the test and contrasts with the daemon tests' existing
`test_support::shell_ready`, which waits for screen output before typing.
The empty running agent in the second log means the watcher looked and found
no recognized agent. It does not establish why, nor that a longer detection
sleep would fix the missing startup precondition.

## Baseline comparison

Unmodified baseline: `06fa623abdcc3210f42f1fb9c4606ba04a22204f`,
`NativeOfferTests.swift` SHA-1 `3051a39a97633711133dd298ef83dd014548897c`.
The unmodified test passed **20/20** sequential runs with the same 16-worker
load generator. Total trial wall time was 118.570 seconds; sampled system
one-minute load ranged from 38.780 to 48.377. These are whole-machine averages,
including unrelated work, not a claim that the generator created 48 workers.
`baseline/results.json` records all 20 outcomes and each sample's 16 live
workers; `baseline/load-pids.json` records their exact PIDs, and
`baseline/load-stopped.json` confirms all 16 were stopped and reaped (SIGTERM).

A pre-fix failure could **not** be shown in these 20 trials. They exercised
CPU pressure, but not the gate's simultaneous many-suite daemon/tmux/GUI
contention. No full gate was run, as the ticket forbids one. A supplementary
plain PTY startup probe also retained early input: input loss is not universal
shell behavior. The specific startup substep responsible in the old gates
remains inferred, rather than established by a reproduced failure.

## Change

Give this test's isolated interactive `/bin/sh` a unique PS1 via its `ENV`
startup file. Before sending anything, poll `terminal screen` through the
real CLI until it contains that prompt. Retry only these reads (including
transient live-inventory lookup failures), bounded by a monotonic 60-second
deadline. Startup output cannot satisfy the unique marker. Send the command
once, then retain the existing running-agent poll and all three assertions:
`runningAgent == "claude"`, `program == "shell"`, and `offers(pane)`.
A subsequent detection failure also captures the shell screen for diagnosis.
The prompt file lives under the already-cleaned scratch home.

## Fixed validation

Fix commit: `a52921a29`.

- `swift test --jobs 4 --filter NativeOfferTests`: **8/8 passed** (`focused.log`).
- `stress.py .../fixed`: **20/20 consecutive passes**, 103.559 seconds total;
  every sample recorded exactly 16 live CPU workers. `fixed/results.json` and
  the twenty `fixed/run-*.log` files contain the outcomes. Whole-machine
  one-minute load ranged from 15.596 to 68.041; unrelated concurrent activity
  is included. The generator itself never exceeded 16 `/usr/bin/yes` processes.
- `fixed/load-pids.json` records all 16 workers. `fixed/load-stopped.json`
  confirms SIGTERM and reaping for every recorded PID, before the full suite.

The baseline and fixed trials used the same foreground Python supervisor and
16-worker generator (`stress.py`), with sequential `swift test --skip-build
--jobs 4 --filter NativeOfferTests.theRealCLINamesTheRunningAgent` invocations.
Each result checks the named test's pass line as well as its exit status, so a
skip or empty filter cannot count as a pass. Builds used four jobs at most.
No full landing gate, formatter, or destructive Git operation was run.

The first clean build required normal process/cache access after compiler
sandbox temporary-file failures. Commands put `/usr/bin` first on PATH because
this machine's `/usr/local/bin/python3` is an incompatible Intel executable.
The early filtered `test.sh` invocation intentionally could not meet the
full-suite count floor, although its one selected test and leak checks passed.
The unique-prompt implementation was also corrected to omit the prompt's
trailing space from the matcher: tmux trims it from screen capture.

Full `apps/macos/test.sh --jobs 4`: **PASS, exit 0**. All five bundles passed:
21 + 3 + 1 + 34 + 1,979 = **2,038 tests**. The Ceremony bundle's 1,979 tests
finished in 157.636 seconds and exceeded the 1,950 floor. The real CLI Claude
offer test also passed inside that full run (4.661 seconds). The tmux leak
guard passed, and the temp-file/defaults leak guard reported nothing left.
See `macos-test-summary.log`; the complete local log is
`/tmp/ov-1323-macos-tests.log`.

Status: complete, with the explicitly documented limit that no pre-fix failure
was reproduced in the capped-load comparison. The startup barrier is now an
observed prompt, rather than an assumption made from creation or send success.
