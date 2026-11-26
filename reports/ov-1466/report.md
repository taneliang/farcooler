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
  versions. On rebased candidate `2ed5cee1b` with tmux 3.6, the named test
  passed in 0.600 seconds and the focused suite passed all 64 tests.
- On GitHub's Ubuntu 24.04 runner, the named test passed in 0.309 seconds
  on `2ed5cee1b` and all four Linux Rust test shards passed.
  [Rust tests (3/4)](https://github.com/taneliang/farcooler/actions/runs/45683358983/job/139452543897)
- macOS arm64, tmux 3.8. On the same candidate, the named test passed in
  0.522 seconds. The focused suite passed all 66 tests, including on the
  final local rerun before landing.
- In GitHub's macOS native shard, the named test passed in 0.858 seconds
  on `2ed5cee1b`.
  [Rust native tests (macOS 1/2)](https://github.com/taneliang/farcooler/actions/runs/45683358983/job/139473766732)
- The focused suites covered daemon workflow admission, caller seats,
  foreground sampling, the process table, and process-read lanes. The
  tmux leak checker passed on both platforms.
- `cargo clippy --locked -p farcooler-daemon --all-targets -- -D warnings`
  passed on macOS. The daemon-layer, file-size-budget, and shared-core lints
  passed.

## Landing-gate follow-up

The landing gate reported 2,761 lines in `crates/client/src/session.rs`
against a 2,744-line ceiling. The inline tests now live in
`crates/client/src/session/tests.rs` under the same Rust module path.
`session.rs` is 2,036 lines in this worktree, and the generated ceiling
is 2,086. The wire quick checks pass, including the file-size-budget
self-test and repository scan.

On macOS, the client suite passed 408 tests, including all 30 in the moved
module, with one existing ignored test. All 66 focused daemon tests passed
with tmux 3.8, including the named regression. Both suites passed the tmux
leak check. Clippy passed with warnings denied for all targets of the
client and daemon, including the client `tailcat` and daemon
`tailcat-helper` and `tailcat` configurations.

In `fcdiag` on Linux aarch64 with tmux 3.6, all 30 moved client tests
passed. The fresh daemon regression runs stopped during compilation. The
VM logged an out-of-memory kill for the first attempt, and the single-worker
retry also ended with rustc receiving SIGKILL. The daemon source matches the
Linux-verified candidate `2ed5cee1b` documented above.

## Main CI

The `CI` push run on `main` for landing commit
`2ed5cee1b46304dcde0a82d9824b9342191a94bd` completed with `success` on
attempt 1. Its Changes job found the successful landing run for the same
SHA and reused that result, so the app jobs were skipped.
[Main CI run 45693542956](https://github.com/taneliang/farcooler/actions/runs/45693542956)
[Changes job log](https://github.com/taneliang/farcooler/actions/runs/45693542956/job/139484929370)

`main` fast-forwarded from `8c85d3179e00de94c02cd89974764c41973c1f67` to
`2ed5cee1b46304dcde0a82d9824b9342191a94bd` on November 26, 2026 (SGT).
Landing CI passed on attempt 2 after retrying two macOS jobs that had been
cancelled before GitHub assigned runners.
[Landing CI run 45683358983, attempt 2](https://github.com/taneliang/farcooler/actions/runs/45683358983/attempts/2)

The prior main push at `8c85d3179` failed on attempt 2. Rust tests (3/4)
logged the same timeout at `admission_tests.rs:573` while the test still
armed the stall on tmux’s `pane_pid`.
[Prior main CI run 45669588871, attempt 2](https://github.com/taneliang/farcooler/actions/runs/45669588871/attempts/2)
