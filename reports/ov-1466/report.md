# ov-1466 · Workflow reads during a stalled argument sample

The Linux regression test timed out because tmux’s `pane_pid` named a launch wrapper outside the tty’s foreground process group, so the argument sampler skipped the PID the test had armed.

During the failing test in the `fcdiag` Linux VM with tmux 3.4, tmux reported
`pane_pid=11359` and `pane_tty=/dev/pts/1` for the second shell pane. The
process table showed the wrapper and its child (wrapper arguments shortened).

```text
  PID  PPID  PGID TPGID TT    STAT COMMAND
11359 11321 11359 11360 pts/1 Ss   sh -c env … /bin/sh -il …
11360 11359 11360 11360 pts/1 S+   /bin/sh -il
```

The Linux sampler calls `stall_foreground_read` immediately before reading
`/proc/<pid>/cmdline`. Both platforms already isolate foreground argument
sampling from the topology reads used for caller admission. The test now waits
for the unrelated pane’s prompt, resolves the foreground PID from its TTY, and
requires the hook to report entry before issuing `workflow.get`. The RPC must
still return a successful response within one second while the stall is held.

## Verification

- Linux aarch64 in `fcdiag`. The named test at `d51b9eb09` reproduced the
  panic `sampler reached the unrelated process's arguments: Elapsed(())`
  with both tmux 3.4 and tmux 3.6. The corrected test passed with both
  versions (1 passed, 0 failed in each run). The same five daemon test
  modules listed below passed with tmux 3.4 (64 tests).
- macOS arm64, tmux 3.8. The named regression passed in the daemon library
  test target on the original run and on recheck (1 passed, 0 failed each time).
  The daemon test modules for workflow admission, caller seats, foreground
  sampling, the process table, and process-read lanes passed (66 tests).
- `cargo clippy --locked -p farcooler-daemon --all-targets -- -D warnings`
  passed on macOS. The daemon-layer, file-size-budget, and shared-core lints
  passed.

## Main CI

The ov-1466 fix is on the ticket branch at
`6eb3b744b1fdd441edf8520608baf64115547886`. The checked main head,
`8c85d3179e00de94c02cd89974764c41973c1f67`, still arms the stall using tmux’s
`pane_pid`. Its CI run was in progress when checked.
[GitHub Actions run 45669588871](https://github.com/taneliang/farcooler/actions/runs/45669588871)

The preceding main CI run failed on this regression in Rust tests (3/4).
[GitHub Actions run 45568814028](https://github.com/taneliang/farcooler/actions/runs/45568814028)

The acceptance check requires a successful `push` run of the `CI` workflow on
`main` whose head SHA is the engine’s landing commit containing the fix. That
commit and run were unavailable while preparing this ticket branch.
