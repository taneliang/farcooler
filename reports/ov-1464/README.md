# ov-1464 · Broken pipe in the ov-1463 build

The November 24 runner records tie the failure to the native hook reply for an
orchestrator message. Times below are UTC.

| Time | Runner evidence |
| --- | --- |
| 13:00:30 | The build-start and native-owner receipts bind ov-1463's build to run `wf1:c4f11fad…`, effect `wf1:67d141cc…`, and terminal `98a90ee3-5637-99a3-1ea2-703c6f3075c8`. |
| 14:55:54.202 | The durable message wake was queued for that terminal. |
| 14:55:54.215–.216 | The wake was claimed and completed with the runner note “Couldn't confirm the engine agent got the message.” |
| 14:55:54.217696 | Line 574 of Canary `farcoolerd.log` logged “the native turn could not be read” for the same effect with `runtime I/O: Broken pipe (os error 32)`. |
| 14:55:54.385 | The native settlement receipt `wf1:bff8faa3…` recorded a failed build. The runner later wrote the process and outcome receipts. |

The writer was `farcoolerd` in `native_followup`. It tried to send the steering
response over the per-hook Unix socket after the `farcooler hook` peer had
closed its read end. The wake and the runner note identify that write. The
launch registration makes Codex's `PreToolUse` the waiting hook and its `Stop`
non-gating. The CLI gives the first reply 400 ms from the start of the hook
errand and drops the socket if the deadline expires. That is the likely
reason for the close. The retained logs show the daemon-side error. The
hook's precise exit reason remains unknown.

Yes, delivering a message could end a working engine agent's turn. With
direction queued, the reply write propagated `EPIPE` into the turn loop. The
runner then quiesced the agent and settled the build as failed. The fix
keeps a disconnected reply local to the hook, records unconfirmed delivery
on the ticket, and lets the live build continue. A disconnected duplicate
Codex Stop reply also leaves the original continuation in place.

## Regression

The test sends a Codex `PreToolUse` frame and closes the socket before
the runner can reply. The native build fixture waits for the test to
observe the durable unconfirmed-delivery note before committing.
With only the test added to `origin/main` at `2ed5cee1b`, the build
settled as `SafeFailureBecause` with the same `Broken pipe (os error 32)`.
With the fix, the same admitted build committed and settled as passed.

The four-worker daemon library run passed 3,403 tests, with six failures
and 18 skipped. All six failures passed in a serial rerun with a smaller
positive nice increment and ARM64 Python first on `PATH`. The first run
hit the macOS niceness ceiling in a scheduling assertion, a four-second
model-refusal deadline, and four fixtures that selected Intel-only Python.
All 11 daemon binary unit tests passed.
`cargo clippy --locked --workspace --all-targets -- -D warnings` passed.

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
