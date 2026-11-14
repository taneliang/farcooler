# ov-1200: vanished reviewer recovery

The test kills its tmux server, then polls the process table until the old pane's tty has no processes. That observation does not reserve the tty: after the server closes it, another test can allocate the same name before recovery takes its own snapshot. Recovery correctly refuses to declare the vanished pane quiescent when it sees any process on the recorded tty, so tty reuse can cause the test to enter `Uncertain` instead of settling the saved verdict. The old bounded poll also fell through without checking whether it ever observed an empty table; lingering processes are a separate possible cause. The test now holds an unused PTY open through recovery, preventing reuse of its recorded tty. It still kills the server and checks both verdicts, saved output, receipt, settlement, and absence of a new reviewer.

## Candidate stress run

On macOS, using the test binary built from commit `39ad29c5f`, six instances ran concurrently, each creating its own real tmux server and PTYs:

```sh
mkdir -p /private/tmp/ov1200-round1-fixed
seq 1 50 | xargs -P 6 -I '{}' sh -c 'target/debug/deps/farcooler_daemon-797f39b52442e3fc workflow_native_review_vanished_server_keeps_output_and_recorded_verdict --nocapture > /private/tmp/ov1200-round1-fixed/{}.log 2>&1'
rg -l 'test result: ok. 1 passed' /private/tmp/ov1200-round1-fixed | wc -l
rg -l 'test result: FAILED' /private/tmp/ov1200-round1-fixed | wc -l
```

Result: 50 passed, 0 failed. Each run exercises both passed and failed verdicts. The commands required execution outside the sandbox, where Git and tmux subprocesses can run.

## Original test under load

A separate source checkout was made with `git archive 73974f89898d716c36595eade2511ba5f8df7a8f | tar -x -C /private/tmp/ov1200-base-round1`. The original was built there with `CARGO_TARGET_DIR=<branch>/target cargo test -p farcooler-daemon --lib --no-run`, then run 50 times with the same `xargs -P 6` command above, changing the log directory to `/private/tmp/ov1200-round1-base` and using that baseline binary. Result: 45 passed, 5 failed. One failure reported:

```text
left: ("agent-review", Uncertain)
right: ("orchestrator-review", WaitingReview)
```

## Diagnostic attribution

In the separate base checkout, a diagnostic-only build printed the tty when the old poll observed an empty `under_tty` result and, when recovery found occupants, printed their PIDs and `ps -o pid,ppid,tty,command -p <pids>`. No synchronization or recovery behavior was changed. With the same six-way workload, a failed run showed:

```text
DIAG poll empty tty=/dev/ttys004
DIAG recovery tty=/dev/ttys004 pids=[35872, 35867]
  PID  PPID TTY      COMMAND
35867 35864 ttys004  fish -c /bin/sleep 30 #farcooler-opening:db2db652-222d-ae5c-278e-b16255cfad55
left: ("agent-review", Uncertain)
right: ("agent-review", Failed)
```

The empty poll followed by occupants on the same tty demonstrates reuse between observations in this failure, rather than a process continuously lingering from before the poll. The shell command is from a concurrent tmux fixture; its marker identifies its opening terminal. The old loop's silent timeout remains a distinct theoretical way to encounter leftover processes, but does not explain this observed empty-then-occupied sequence. The instrumented source was confined to the temporary checkout.

The diagnostic run completed with 44 passes and 6 failures out of 50; the added logging was used only for attribution, not as the baseline failure count.
