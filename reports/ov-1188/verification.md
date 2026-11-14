# ov-1188 verification

Read `farcooler task show ov-1188` using the installed Canary CLI before editing.

The first regression test published a real engine pane in its private tmux
server and addressed its ticket through `message.send`. Before the fix it failed:

```text
the engine's live agent must be reachable: Err(Conflict { what: "nobody" })
```

The engine now registers a live steering capability for each native effect,
including each ticket of a shared build or group gate. Plan reads project those
agents and their ticket references. Stop acknowledges only after native process
quiescence; pause/resume preserve the current run, process tree, session, and
active-time deadlines. Watching keeps its existing read-only route. Landing
continues through the engine's existing settlement path.

Messages stay in the existing durable queue and are claimed at the current
session's Stop hook. Codex and Claude receive a blocking Stop response with the
message as their continuation, before the engine settles the step. A replacement
step never receives a message addressed to the old terminal. Answer-note CLI
output explains the explicit message needed to deliver the new direction.

The real-PTY continuation test covers both Codex and Claude stand-ins and checks
that the workflow state stays unchanged. The pause test checks that the same
processes stop writing, that quiet-period accounting is frozen, and that active
time resumes. Ten consecutive stress iterations of both tests passed.

Pause investigation found that tmux automatically continues a stopped pane
leader. Native launch now retains a waiting shell as that leader and suspends
only the agent and its descendants. The pause scan verifies kernel stop state
and tolerates a short-lived child exiting during the scan.

No `cargo fmt` was run.

## Final checks

Tests run with this task's inherited `FARCOOLER_*` launch variables removed
(except the public WorkOS client ID), so spawned hook binaries used their test
sockets. The first broad attempt inherited the live native-hook socket and was
stopped after that environment contamination was identified.

- Full affected-package run (`farcooler-daemon`, `farcooler-cli`,
  `farcooler-client`, `farcooler-protocol`): 4,823 passed, one intermittent native
  vanished-server recovery failure, 18 skipped. The tty-sensitive case passed
  20 isolated stress iterations. It now reserves all nextest threads, as the
  existing tty-number-sensitive tests do, so another test cannot reuse its dead
  tty between the quiescence wait and reconciliation. Its production recovery
  check and test assertions are unchanged.
- Final complete daemon unit suite, with that scheduling: **3,073 passed**,
  13 skipped, under `scripts/tmux-leak-check.py`.
- Final focused CLI and steering run: **115 passed**, including the recovery
  case, stop while paused, both harness continuations, agent rows, and RPC
  authorization. No leaked test servers were reported.
- Native continuation and pause/resume stress: **10 of 10 iterations passed**.
- `cargo clippy -p farcooler-daemon -p farcooler-cli -p farcooler-client
  -p farcooler-protocol --all-targets -- -D warnings`: passed.
- Affected-package doctests: passed.
- File-size budget, protocol capability checks, swallowed-call lint, copy lint,
  visual-token lint, doc-comment check, and whitespace checks: passed.
