# ov-1357 — native builders during landing gates

## Fix round 1: reproduced pane loss

The earlier change fixed an unsafe pause but had not reproduced a lost pane.
This round isolates a second fault: `TmuxServer::pane_life` mapped **every**
unsuccessful tmux client to `PaneLife::Gone`. Two such results made
`coding_native::native_turn` return “the native session was lost without a
receipt”. `run_native` then called `quiesce_native`, which validated the still-live
pane and killed its processes before closing the window; `Host::drop` finally
reaped its private tmux server. The engine could therefore close a live
builder itself based solely on failed observation clients.

The fix accepts `Gone` only for an explicit missing pane or socket reported
by a normally exiting tmux client. Other failures, including signaled clients,
return `TmuxUnavailable`; the existing bounded unreadability handling retries
them and keeps the builder alive. Unexpected failures now record their terminal,
client exit status, and up to 1000 characters of stderr, so a recurrence retains
the evidence that the old logs discarded. Actual dead panes, foreign tags, and
explicitly missing panes retain their existing handling.

The native gate regression now launches a child test harness with a private tmux
shim. Only that harness's pane-liveness clients fail; creation, live inventory,
fanout, input, and teardown still use real tmux. Each of four builders receives
two client failures, with a gate arriving mid-build or present before launch,
for both Codex and Claude hook fixtures. It checks the same pane remains live,
the agent is stopped when started, and the build commits and passes after the
gate releases it. A separate tmux regression independently confirms the pane is
live after each failed probe and lets its original program finish.

No paid agent session or `scripts/landing-gate` was run. An empty, unauthenticated
private Codex TUI was also checked at its login screen without submitting a
prompt. The prior supervisor identity and pause/resume fix is retained.

## Historical attribution

The six retained Canary warnings below prove the symptom, but retain neither the
raw probe result nor the pane's exit signal. The injected failures establish a
specific route to that exact error and engine-caused pane teardown; they do not
prove which tmux error happened on Nov 19. The historical trigger remains
unconfirmed. A private reproduction of the production fish launch wrappers,
using only `sleep`, left the pane live with both old and new stop sequences;
fish cannot be named as the pane closer from that experiment. The existing
process-guard tests also keep a bystander alive through gate cancellation and
normal cleanup. No broader process cleanup was changed on speculation.

## Failing-first evidence

The new lower-level tests failed against the original probe implementation:

```text
Summary [0.110s] 2 tests run: 0 passed, 2 failed
an unsuccessful probe must be unknown, not a closed pane that makes the runner tear down its host
```

For the native regression, only `probe` was restored to its original behavior;
the earlier supervisor/pause fix remained in place. The real build then failed
in 1.21 seconds while its gate was held:

```text
gate ended the builder: Ok(SafeFailureBecause("The agent's turn ended before it finished: the native session was lost without a receipt. Its work stays in the worktree for the step's next attempt, a retry or a send-back."))
Summary [1.234s] 1 test run: 0 passed, 1 failed
```

The fix was restored before final verification. The exact commands are the final
commands below with the corresponding test names selected. Local transcripts:
`/tmp/ov1357-r1-red.log`, `/tmp/ov1357-r1-native-red.log`, and
`/tmp/ov1357-r1-final-tests.log`.

## Retained pause fix

Separately confirmed and fixed: the gate's native pause included the tmux pane supervisor.
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

## Retained historical log evidence

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

## Validation commands

Builds used `scripts/heavy`, no debug information, and no incremental compilation.
The daemon binary was built first for native output fanout. The final focused
selection covers tmux probe/absence handling, native builders during gates,
yielding identities, manual pauses and deadlines, and configured builders:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 /usr/bin/python3 scripts/heavy --light --load-limit 30 -- \
  cargo nextest run -p farcooler-daemon -p farcooler-tmux --lib -E \
  'test(pane_life::) | test(pane_read::tests::) | test(yielding::tests::) | test(workflow_runtime::coding_native::unread_tests::) | test(workflow_runtime::native_control::tests::) | test(workflow_coding_native_real_pty_normalized_hooks_and_quiescence) | test(a_running_builder_is_paused_while_a_landing_gate_runs_and_resumes_after)'
```

The native quiescence ordering tests are selected separately with
`test(workflow_runtime::coding_native::quiesce_order_tests::)`.

- Final focused Rust selection: 33 passed; native quiescence ordering: 3 passed.
- Existing process-guard bystander tests: 2 passed (normal completion and gate cancellation).
- `PATH=/usr/bin:/bin:/opt/homebrew/bin:$HOME/.cargo/bin sh scripts/lint-gate.sh`: passed.
- `/usr/bin/python3 scripts/file-size-budget.py`: passed.
- `git diff --check`: passed.
