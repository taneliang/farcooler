# ov-1464 · Broken pipe in the ov-1463 build

The November 24, 2026 runner records tie the failure to the native hook reply
for an orchestrator message. Times below are UTC.

| Time | Runner evidence |
| --- | --- |
| 13:00:30 | The build-start and native-owner receipts bind ov-1463's build to run `wf1:c4f11fad…`, effect `wf1:67d141cc…`, and terminal `98a90ee3-5637-99a3-1ea2-703c6f3075c8`. |
| 14:55:45.961 | The Codex rollout for session `01a20da4-c6a4-7961-9a5e-569f3b58f014` records a `wait` tool call at line 1520. There is no result, Stop, or hook-exit record before the rollout ends. |
| 14:55:54.202 | The runner database's `message_wakes` row records the wake queued for that terminal. |
| 14:55:54.215–.216 | The wake was claimed and completed with the runner note “Couldn't confirm the engine agent got the message.” |
| 14:55:54.217696 | Line 574 of Canary `farcoolerd.log` logged “the native turn could not be read” for the same effect with `runtime I/O: Broken pipe (os error 32)`. |
| 14:55:54.385 | The native settlement receipt `wf1:bff8faa3…` recorded a failed build. The runner later wrote the process and outcome receipts. |

The writer was `farcoolerd` in `native_followup`. It tried to send the steering
response on the hook connection to `n-98a90ee35637.sock` after the
`farcooler hook` peer had closed its read end. The wake completion note is
written on the reply error path before that error propagates out of the native
turn. This places the disconnect before the runner quiesced the agent.

The launch override and the worktree hooks both register Codex `PreToolUse`
with `--gating` and a 70-second outer timeout. The CLI makes a native Codex
`Stop` wait as well, using the private socket recorded in the process receipt.
For either waiting hook, the CLI gives first contact 400 ms. If it has written
the frame and the reply deadline expires, it closes the socket and exits 0
with empty stdout. The runner may still process the buffered frame and attempt
a reply.

That deadline explains how a live agent can leave the runner with a closed
reply socket. For wf1 specifically, the retained evidence stops at the failed
write. The settlement records zero hook frames on the error path, the captured
PTY output contains no hook-exit diagnostic, and the local Codex log database
starts on November 25 at 20:31:13. The rollout identifies the last tool call
but does not timestamp its hook invocation or exit. Establishing the historical
close reason requires a client-side record tied to the failed hook connection,
with the event, timestamp, and reason the process stopped reading. The wake
claim time alone gives no bound on how long the hook had been waiting.

Yes, delivering a message could end a working engine agent's turn. The send
RPC queued the wake. The runner claimed it 13 ms later, and the delivery
write propagated `EPIPE` into the turn loop. The runner then quiesced the
agent and settled the build as failed. The fix keeps a disconnected reply
local to the hook, records unconfirmed delivery on the ticket, and lets the
live build continue. A disconnected duplicate Codex Stop reply also leaves
the original continuation in place.

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

The niced daemon library and binary unit suites passed all 3,420 selected
tests with four Nextest workers and 18 skipped.
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
