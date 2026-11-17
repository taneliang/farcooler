# ov-1323 — live lookup and foreground observation races

## Cause and evidence

The test crosses two independently scheduled boundaries without waiting long
enough for their actual answers. A created pane (or even its visible prompt)
does not guarantee that a new CLI's inventory read will complete within tmux's
one-second command deadline. A successful send/list RPC does not guarantee that
the daemon has completed a foreground observation after that send. These are
separate races; the earlier report's claim that startup lost input was not
established and is withdrawn.

Both Nov 17 gate logs were read in `/Users/e-liang/Dev/overnight`:

- `ov-1308/gate-swift-tests.log:6999-7000`: `terminal send` exited 1 with
  `error: resource not found`, after 4.590 seconds. The local send path in
  `crates/cli/src/main.rs` opens `Runtime`, resolves the terminal against its
  inventory, and only then calls `send_input`. `Runtime::open_in` refreshes a
  NEW inventory. `LiveInventory::refresh` tries `list-panes` twice, with a
  150 ms pause; each has a one-second deadline. If both reads fail, it publishes
  an unavailable snapshot with no panes. `resolve_terminal` then raises
  `NotFound` before any input is sent. This error is therefore also possible
  for a live pane; it does not prove a missing pane or a shell startup problem.
- `ov-1320/gate-2-swift-tests.log:7354-7360`: send succeeded, but the final
  successful list still carried `runningAgent: ""`; `offers` failed too.
  The test failed after 47.992 seconds (`:7446`). Peak system load was
  `[34.3223, 20.3574, 12.9644]` (`:7590-7594`). Lists serve the watcher's last
  observation; they do not force a new sample. In `Watcher::sample`, the daemon
  reads the foreground process table and awaits `HostWalk::read`'s socket walk
  (`lsof`) BEFORE publishing `running_agent`. That subprocess has no deadline.
  The old loop exhausted 60 lists separated by 500 ms, regardless of whether
  this work had finished. Empty means the last completed observation found no
  agent; it does not establish what is running now or that input was lost.

The historical gate logs contain neither tmux timing diagnostics nor the
foreground argv. They cannot uniquely identify which original read was delayed.
The causal paths above were therefore tested with controlled scheduling delays,
rather than claiming that the old logs prove a particular lost keystroke or a
particular historical `lsof` duration.

## Controlled reproduction with the real CLI and daemon

`probe.py` delays real subprocesses, never fabricates their output. Its
`round1-path-probe/summary.json` records:

- At 3.600 s, two timed-out inventory reads yielded the exact CLI error
  `resource not found`. At 3.609 s, direct tmux inventory still showed the
  terminal's exact tag, `pane_dead=0`, and a live bash shell.
- At 3.650 s, a normal send succeeded. At 4.382 s, `ps` showed PID 67036,
  PGID 67036, `SN+`, `/tmp/fco-probe-65333/claude 600` on `ttys016` while the
  watcher socket read was held. Input had executed and was in the foreground.
- At 38.711 s, the old 60-read loop still returned `runningAgent: ""`.
  After the held read completed, the SAME process was reported as `claude` at
  49.362 s, without resending input.

`fault-test.py` then reproduced these paths in the **actual Swift test**, with
exactly 16 CPU workers running alongside each invocation. The before runs used
revision `87b9e847f`, including its unique-prompt barrier; thus they also prove
that the barrier alone cannot fix either demonstrated race. It retains the
original single-send and 60-read detection behavior from the unmodified test.

| Controlled delay | Before revision | After revision |
| --- | --- | --- |
| Delay both inventory reads on the first send invocation | FAIL: `resource not found`, 5.710 s | PASS: 6.988 s; exactly one `send-keys` |
| Hold the first post-send watcher `lsof` for 45 s | FAIL: empty `runningAgent` and `offers`, 48.311 s | PASS: 50.820 s; exactly one `send-keys` |

The before watcher run records PID 57775 in foreground group 57775 (`SN+`),
`/tmp/fco-985ED4/bin/claude 600`, while lists keep returning empty. Its failure
also captures the already-executed command on screen. This reproduces ov-1320's
symptom without any lost input. `round1-before-{lookup,watcher}/swift.log` and
`faults.jsonl` contain the failures and live-process evidence. Matching
`round1-after-*` directories contain the same controlled delays after the fix.

These controlled delays model scheduling stalls; they are not a claim that a
naturally occurring 45-second `lsof` stall was observed in either historical
gate. The earlier unmodified baseline passed 20/20 under CPU load alone
(`baseline/results.json`); CPU pressure without the gate's other simultaneous
contention did not reproduce a natural failure. No full gate was run.

## Fix

Commit `8cd8f76` changes only `NativeOfferTests.swift`:

1. Retain the isolated shell's unique prompt as an explicit startup precondition.
2. Retry a send only when it exited 1, without timeout/cancellation/launch
   failure, with empty stdout and final stderr line `error: resource not found`.
   This is the resolution failure proven to occur before writing. All other
   errors, including ambiguous write timeouts, fail immediately. The retry has
   a monotonic 60-second bound. The controlled lookup run records two delayed
   reads followed by exactly ONE actual `send-keys` operation.
3. Poll the actual decoded `runningAgent == "claude"` condition until a
   monotonic 60-second deadline, rather than exhausting a sample count while
   the watcher is still busy. Preserve the exact `runningAgent`, `program ==
   "shell"`, and `offers` assertions, plus the failure screen diagnostic.

## Validation

- `swift test --package-path apps/macos --jobs 4 --filter NativeOfferTests`:
  **8/8 passed** after the change (`round1-focused.log`).
- `stress.py .../round1-fixed`: **20/20 consecutive passes**, 113.586 s total;
  each sample recorded exactly 16 live workers and verified the named test's
  pass line, not merely exit 0. `round1-fixed/results.json` and `run-*.log`
  preserve all outcomes. Whole-machine one-minute load ranged 28.741–62.836;
  that includes unrelated concurrent work, while our generator remained capped
  at 16. `load-stopped.json` confirms all 16 were SIGTERMed and reaped before
  the final full suite.
- Final `CARGO_BUILD_JOBS=4 apps/macos/test.sh --jobs 4`: **PASS**. All five
  bundles passed: 21 + 3 + 1 + 34 + 1,979 = **2,038 tests**. Ceremony finished
  in 168.321 s; this real CLI test passed inside it in 7.178 s. The test floor,
  tmux leak guard, and temp-file/preferences guard passed. See
  `round1-macos-summary.log`; full log `/tmp/ov-1323-round1-macos-final.log`.
  The pre-revision clean build/full suite passed too
  (`/tmp/ov-1323-round1-macos-initial.log`).

All load runs use a foreground Python supervisor, start exactly 16 `/usr/bin/yes`
workers, record their PIDs before testing, and SIGTERM and reap those recorded
PIDs in `finally`. Each run directory has `load-pids.json` and
`load-stopped.json`. Builds use four jobs maximum. No full gate, formatter,
Git stash/reset/clean/amend, or blanket Git add was used.

Status: complete. The historical logs cannot identify the exact delayed syscall;
the two failure paths and the resulting fix are demonstrated by the controlled
real-CLI/daemon reproductions above.
