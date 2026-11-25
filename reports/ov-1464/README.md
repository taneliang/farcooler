# ov-1464 · Broken pipe in the ov-1463 build

The November 24, 2026 runner records tie the failure to the native hook reply
for an orchestrator message. Times below are UTC.

| Time | Runner evidence |
| --- | --- |
| 13:00:30 | The build-start and native-owner receipts bind ov-1463's build to run `wf1:c4f11fad…`, effect `wf1:67d141cc…`, and terminal `98a90ee3-5637-99a3-1ea2-703c6f3075c8`. |
| 14:55:45.961 | The Codex rollout for session `01a20da4-c6a4-7961-9a5e-569f3b58f014` records a `wait` tool call at line 1520. There is no result, Stop, or hook-exit record before the rollout ends. |
| 14:55:54.202 | The runner database records the only `message_wakes` row addressed to that terminal. Its note actor is `manager`. |
| 14:55:54.215–.216 | The wake was claimed and completed with the runner note “Couldn't confirm the engine agent got the message.” |
| 14:55:54.217696 | Line 574 of Canary `farcoolerd.log` logged “the native turn could not be read” for the same effect with `runtime I/O: Broken pipe (os error 32)`. |
| 14:55:54.385 | The native settlement receipt `wf1:bff8faa3…` recorded a failed build. The runner later wrote the process and outcome receipts. |

The writer was `farcoolerd` in `native_followup`. It tried to send the steering
response on the hook connection to `n-98a90ee35637.sock` after the
`farcooler hook` peer had closed its read end. The wake completion note is
written on the reply error path before that error propagates out of the native
turn. This places the disconnect before the runner quiesced the agent.

## Hook disconnect

The runner reached `native_followup` after decoding a complete hook frame
for the bound Codex session. The wake completion note narrows the write to
that function's steering reply. Its only eligible events were `PreToolUse`
and `Stop`. The `wf1:bff8faa3….launch-hooks` receipt and the worktree hooks
both register `PreToolUse` with `--gating` and a 70-second Codex command
timeout. The CLI makes `Stop` wait when the launch environment supplies
the private socket. A routine fire-and-forget lifecycle hook cannot
explain this write.

On either waiting hook, the 400 ms first-contact deadline covers reading
stdin, connecting, writing the frame, and receiving the first reply. The
runner reaches the steering write before it writes anything else on that
connection. If the hook exits normally after sending the frame while that
first reply is still pending, the deadline accounts for the close. The CLI
exits 0 with empty stdout. The runner can still process the complete frame
after the socket closes. A hook process terminated while waiting can leave
the same frame and the same `EPIPE`.
[Hook client](../../crates/cli/src/hook.rs) ·
[Native turn loop](../../crates/daemon/src/workflow_runtime/coding_native.rs)

The retained wf1 records establish the disconnect before the reply. The
actual close reason remains unverified.

- `farcoolerd.log` records the resulting I/O error without a hook event or
  connection timestamp. The error path writes the native settlement with
  zero hook frames, discarding the in-memory frames.
- The process receipt's captured PTY output contains no hook-exit diagnostic.
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

The timeout regression reproduces the normal exit path and its engine
failure. Confirming that path for wf1 requires a record tied to the failed
connection that identifies why the hook stopped reading.

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

The test runs the real `farcooler hook` CLI for Codex `PreToolUse` against the
native runner's private socket with a steering message pending. It pauses
polling the turn until the hook exits successfully after its 400 ms deadline
with empty stdout, then lets the runner process the queued frame. The native
build fixture waits for the durable unconfirmed-delivery note before
committing.

On `origin/main` at `2ed5cee1b` with just the test and its fixture added, the
build settled as `SafeFailureBecause` with the same
`Broken pipe (os error 32)`. With the fix, the same admitted build recorded
the unconfirmed delivery, committed, and settled as passed.

The niced four-worker daemon library and binary unit run recorded 3,412
passes, eight failures, and 18 skipped. All eight failures passed in a
serial rerun with ARM64 Python first on `PATH`. The hook timeout regression
also passed in that rerun.
`cargo clippy --locked --offline --workspace --all-targets -- -D warnings`
passed.

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
the worktree.
