# ov-1357 — native builders during landing gates

Confirmed and fixed: the gate's native pause included the tmux pane supervisor.
`run_native` registered only `Target::Tty`; `yielding::signal` stopped every
process on that tty. tmux automatically continues a stopped pane leader's
process group, so the gate and tmux repeatedly stopped and continued the
builder instead of holding it paused. The manual native pause already excluded
the supervisor for this reason; the gate path did not follow that rule.

The fix records the native supervisor's PID and start identity along with its
tty, excludes that waiting supervisor from pause/resume signals, and verifies
that the same supervisor still owns the tty before signaling. The agent and its
children pause, their retained pane stays live, and they resume after the last
gate. Legacy `Tty` restart records remain readable. Configured command builders
retain their existing process-group behavior.

## Evidence

The new native builder regression was run against the old implementation first.
After two seconds of an active gate it failed with the actual process state:

```text
the gate must actually pause the native builder: SN+
test result: FAILED. 0 passed; 1 failed
```

A separate private tmux/process-table experiment also found the shell and its
child running again after repeated SIGSTOPs. It used a private socket, not an
installed daemon or a live builder. No `scripts/landing-gate` invocation was used.

After the change, the same regression passes. It exercises Claude and Codex
hook fixtures over real retained tmux PTYs, with a gate arriving during a build
and with a build starting during an existing gate. It verifies a live pane,
a stopped agent when started, and a successful build after release. It launches
no paid agent sessions. A second live test verifies that an unrelated pane
continues writing during a valid pause, wrong start identities and mismatched
ttys receive no signals, the identity is persisted, and a hand pause survives
the end of a gate until explicitly resumed.

The retained Canary log corroborates the ticket's historical symptom. On
2026-11-18 UTC it records `the native session was lost without a receipt` at:

| UTC | Effect prefix |
| --- | --- |
| 16:58:59.082739 | a5e520d40763 |
| 16:59:23.334963 | 6e0d733ee4ce |
| 16:59:37.481562 | 51e92b2d3532 |
| 16:59:59.529184 | bd4ba5b209cb |
| 17:01:33.907536 | 965406e1a02e |
| 17:01:44.496717 | 23e37fcf7924 |

Source: `~/Library/Logs/com.farcooler.FarCoolerCanary/farcoolerd.log`.
The first effect's retained owner identifies ov-1352, terminal
`0c971027-732f-a9f6-79e3-3bfab05bba43`, pane `%0`, tty `/dev/ttys020`.
The concurrent group-4321 gate receipt reports a successful 1813.45-second run.
A kernel-log query for memory-pressure kills during 00:54–01:06 local returned
no matching events.

## Limits of the historical attribution

The reproduced failure is the broken pause, not the original pane death.
The old implementation also left the synthetic pane alive. The historical
receipts do not retain tmux's exit status, terminating signal, or the raw pane
probe output. Consequently this evidence does **not** establish which process
closed those six historical panes, or prove that correcting the pause alone
eliminates that exact failure. The root cause of the pause failure is established;
the ticket's stronger historical pane-loss attribution remains open. No broad
cleanup or kill behavior was changed on speculation.

## Validation

- Targeted daemon selection: 22 tests passed, covering yielding, native pane
  reads and gate survival, manual pause/deadline accounting, native PTY/hooks/
  quiescence, and configured builders yielding to a landing gate.
- `PATH=/usr/bin:/bin:/opt/homebrew/bin:$HOME/.cargo/bin sh scripts/lint-gate.sh`: passed.
- `/usr/bin/python3 scripts/file-size-budget.py`: passed.
- `git diff --check`: passed.

Builds used `scripts/heavy`, no debug information, and no incremental compilation.
The native tests require the locally built daemon binary for output fanout.
The final targeted command was:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 /usr/bin/python3 scripts/heavy --light --load-limit 30 -- \
  cargo nextest run -p farcooler-daemon --lib -E \
  'test(yielding::tests::) | test(workflow_runtime::coding_native::unread_tests::) | test(workflow_runtime::native_control::tests::) | test(workflow_coding_native_real_pty_normalized_hooks_and_quiescence) | test(a_running_builder_is_paused_while_a_landing_gate_runs_and_resumes_after)'
```
