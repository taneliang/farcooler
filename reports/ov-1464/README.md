# ov-1464 · Broken pipe in the ov-1463 build

The November 24, 2026 runner log and wake receipts survived the restart. They
identify the failed write as the daemon's steering reply for an orchestrator
message. The hook exit record that would explain why the socket closed is
unavailable. Times below are UTC.

| Time | Runner evidence |
| --- | --- |
| 13:00:30 | The build-start and native-owner receipts bind ov-1463's build to run `wf1:c4f11fad…`, effect `wf1:67d141cc…`, and terminal `98a90ee3-5637-99a3-1ea2-703c6f3075c8`. |
| 14:55:45.961 | The Codex rollout for session `01a20da4-c6a4-7961-9a5e-569f3b58f014` records a `wait` tool call at line 1520. There is no result, Stop, or hook-exit record before the rollout ends. |
| 14:55:54.202 | The runner database records the only `message_wakes` row addressed to that terminal. Its note actor is `manager`. |
| 14:55:54.215–.216 | The wake was claimed and completed with the runner note “Couldn't confirm the engine agent got the message.” |
| 14:55:54.217696 | Line 574 of Canary `farcoolerd.log.1` logged “the native turn could not be read” for the same effect with `runtime I/O: Broken pipe (os error 32)`. |
| 14:55:54.385 | The native settlement receipt `wf1:bff8faa3…` recorded a failed build. The runner later wrote the process and outcome receipts. |

The writer was `farcoolerd` in `native_followup`. It tried to send the steering
response on the hook connection to `n-98a90ee35637.sock` after the
`farcooler hook` peer had closed its read end. The wake completion note is
written on the reply error path before that error propagates out of the native
turn. This places the disconnect before the runner quiesced the agent.

## Hook disconnect

The wake completion note narrows the writer to the steering response body
or its terminating newline in `native_messages.rs:76` at base `2ed5cee1b`.
The eligible events were `PreToolUse` and `Stop`. The launch-hooks receipt
and worktree configuration register Codex `PreToolUse` with `--gating` and
a 70-second Codex command timeout. The CLI makes `Stop` wait on the private
socket. A routine lifecycle hook cannot reach the steering write.
[Steering reply](../../crates/daemon/src/workflow_runtime/native_messages.rs#L59) ·
[Socket write](../../crates/daemon/src/workflow_runtime/native_messages.rs#L14) ·
[Hook registration](../../crates/daemon/src/hook_install/native_host.rs#L35) ·
[Stop gating](../../crates/cli/src/hook.rs#L117)

| Mechanism | Code and incident evidence |
| --- | --- |
| A waiting hook times out after sending the frame | [hook.rs:61](../../crates/cli/src/hook.rs#L61) sets the 400 ms deadline. [hook.rs:275](../../crates/cli/src/hook.rs#L275) drops the first contact if its deadline expires while the first reply is pending. The runner can read the complete frame after the CLI closes. A hold timeout requires an initial reply, which this path never sends. |
| The hook process is terminated after sending the frame | A signal or a harness timeout can close the process's socket while it waits in [hook.rs:329](../../crates/cli/src/hook.rs#L329). The retained records have no matching hook exit. |
| A non-gating `PreToolUse` exits immediately | [hook.rs:226](../../crates/cli/src/hook.rs#L226) advertises updated input even without gating, and [hook.rs:325](../../crates/cli/src/hook.rs#L325) closes without waiting. The installed wf1 hook used gating. The failed frame does not survive to establish which hook registration sent it. |
| Codex sends a duplicate `Stop` and that peer disconnects | Base `coding_native.rs:420–421` replayed the previous continuation with fatal writes. The [replay branch](../../crates/daemon/src/workflow_runtime/coding_native.rs#L420) bypasses wake completion, so it cannot explain the failed-delivery note in wf1. |
| An empty or informational reply goes to a closed peer | The [empty reply](../../crates/daemon/src/workflow_runtime/coding_native.rs#L436) ignores the write error. The [informational context reply](../../crates/daemon/src/watch/answer_wake/engine.rs#L69) handles it locally. Neither ends a turn through this error. |

The retained wf1 records establish the disconnect before the reply. The
actual close reason remains unverified.

- `farcoolerd.log` records the resulting I/O error without a hook event or
  connection timestamp. The error path writes the native settlement with
  zero hook frames, discarding the in-memory frames.
- The process receipt's captured PTY output contains no hook-exit diagnostic.
  Its exit value of `1` is the runner's translation of the failed native
  settlement. The hook subprocess has no recorded exit status.
  The rollout's final `wait` call precedes the error by 8.257 seconds.
  The Codex thread history still marks the turn in progress and has no
  completion for that call or hook result. The 8.257 seconds measure the
  interval from the tool-call record, whose link to the failed frame is
  unknown.
- The oldest retained event in the local Codex log database is November 25
  at 20:31:13 UTC. There are zero log rows for this session.
- The standard macOS unified-log query has no records for the runner,
  Codex, or related kernel messages in the incident window. Its earliest
  matching record is November 25 at 07:00:06 UTC.
- The retained macOS Special trace `0000000000001bb3.tracev3` contains
  1,221 records in the incident window (14:54:30–14:56:00 UTC). No decoded
  entry in that window identifies the runner, Codex, or the hook process.
- The rotated `system.log.2.gz` has one entry in the incident minutes
  (22:54–22:56 local time) with no runner, Codex, tmux, or process
  termination marker. The user and retired crash report directories have
  no report for a matching process at the time of the disconnect. Time
  Machine lists no local snapshots to recover the earlier Codex log
  database from.

## Message delivery

Yes, delivering a message could end a working engine agent's turn. The send
RPC only queued the wake. The runner claimed it 13 ms later and attempted
delivery through the hook. The empty-reply path already tolerated a
disconnected hook. The steering reply propagated `EPIPE` into the turn loop,
so the runner quiesced the agent and settled the build as failed. The fix
keeps a disconnected reply local to the hook, records unconfirmed delivery
on the ticket, and lets the live build continue. A disconnected duplicate
Codex Stop reply also leaves the original continuation in place.

## Regression

The regressions queue a manager message against an admitted Codex build.
They close the client socket after the frame is on the wire and before
the runner polls the reply.

- The real gating `PreToolUse` CLI exits successfully with empty stdout
  after its 400 ms deadline.
- A subprocess that has sent a waiting `Stop` frame is killed before the
  reply. The build still has no commit at this boundary and can continue.
- The real non-gating `PreToolUse` CLI sends a frame and exits without
  reading the reply.
- A connected `Stop` gets a continuation, a disconnected duplicate hits
  the replay, and a third copy can still read the original continuation.

The first three cases require the durable unconfirmed-delivery note. Each
case requires the same admitted build to commit and pass.
[Regression tests](../../crates/daemon/src/workflow_runtime/coding_native_delivery_tests.rs#L118)

All four cases settled with `SafeFailureBecause` and
`Broken pipe (os error 32)` on base `2ed5cee1b` with only the regressions
and their fixture added. All four passed with the production fix. The
landing base `ebc4b7867` has identical pre-fix native reply and CLI hook code.
All four passed again after the rebase using four niced test workers.

Before the rebase, the daemon library and binary unit run used four niced
workers and reported 3,422 passes, one failure, and 18 skips. The sole failure,
`workflow_execution_phases_follow_claims_and_slots`, reported uncertain
worktree provisioning and passed in a serial rerun. On the landing base, the
killed-hook fixture uses the system Python interpreter, as the fake native
agent already does. This avoids selecting an x86-only interpreter from PATH
on an Arm runner.

`cargo clippy --locked --offline --workspace --all-targets -- -D warnings`
passed on both bases with six build jobs.

After the rebase, the complete daemon library and binary unit run reported
3,415 passes, eight failures, and 18 skips. All eight passed in a serial
rerun. Four fixtures had selected the x86-only Python from PATH. The
builder-versus-gate test needed headroom below the niceness ceiling to
measure a difference. The other three passed in isolation.

## Recovery after a genuine interruption

Automatically relaunching a genuinely ended agent under the same admitted
attempt is unsafe. The binding identifies one invocation, its private
terminal and launch evidence, and an immutable settlement. An I/O error can
also leave process ownership or the outcome of a command unknown. Even
after proving quiescence, a fresh launch could repeat a command that already
changed the worktree or external state. Resuming in the same slot would
require a recovery protocol that proves the old owner is gone, reconciles
its effects, and gives the new invocation a separate identity. Genuine
interruptions retain the existing settlement and retry rules, which preserve
the worktree. The runner quiesces the terminal before writing its settlement.
Reconciliation reads that settlement for the original binding and can account
for an interrupted owner under the existing receipt rules.
[Quiescence and settlement](../../crates/daemon/src/workflow_runtime/coding_native.rs#L557) ·
[Interrupted owner reconciliation](../../crates/daemon/src/workflow_runtime/coding_native.rs#L587)
