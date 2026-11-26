# ov-1469 · Native agent startup

The engine counted an open, live tmux pane as a running session before any
hook arrived. A silent agent can now be seen as **Stalled** after 60 seconds
and has a Needs You item with the ticket and startup diagnostic. The first
accepted session hook clears the startup stall. A known landing pause is
included in the diagnosis while the gate holds the builder.

The macOS native launch resolves the executable using the login shell’s
PATH and checks it for `com.apple.quarantine` immediately before exec.
A quarantined executable is refused. The engine retains the diagnostic on
the workflow item and uses the same Needs You entry, including during
automatic retries. For a Codex npm launcher, it also checks Node and resolves
the native vendor package using that Node's architecture and package lookup.
It checks both the current `bin/codex` and older `codex/codex` layouts for
the selected architecture. For the standalone Cursor launcher it checks the
bundled Node executable. Custom launch scripts receive the entry-point
check. [Current Codex launcher](https://github.com/openai/codex/blob/rust-v0.172.0/codex-cli/bin/codex.js),
[older Codex launcher](https://github.com/openai/codex/blob/rust-v0.100.0/codex-cli/bin/codex.js).

## macOS process responsibility

**Yes, the native hold changes responsibility when macOS attributes child
processes to the launching app.** On macOS 27.2.1 (26B131), a scratch
background app started a private tmux server. The same small Mach-O
executable ran once directly in a pane and once through the checkout’s
`farcoolerd --native-hold`. The executable queried libSystem’s
`responsibility_get_pid_responsible_for_pid` for itself and its parent.

| Launch | Scratch app PID | Pane PID | Probe PID | Probe parent PID | Responsible PID |
| --- | ---: | ---: | ---: | ---: | ---: |
| Direct | 70075 | 70089 | 70089 | 70088 (tmux) | 70075 (app) |
| Native hold | 70075 | 70099 | 70107 | 70106 (hold) | 70106 (hold) |

The pane process in the held case was the relay. The relay uses
`responsibility_spawnattrs_setdisclaim` before spawning the hold. macOS
then attributes the hold and its agent to that new hold PID. In a separate
headless probe on the same machine, macOS returned each executable’s own
PID (66122 direct, 68400 held). Launching the scratch app via Launch
Services gave the probe the inherited responsibility needed for the
comparison.

The app probe measured process attribution. The regression tests cover a
real quarantine xattr on a scratch executable, a clean npm launcher that
points to a quarantined Mach-O and reports its path in Needs You, prompt
submission through the hold, and a private builder resuming after a landing pause. These
results leave the cause of the four historical Canary stalls unconfirmed.
The probe used a scratch home and private tmux sockets.
