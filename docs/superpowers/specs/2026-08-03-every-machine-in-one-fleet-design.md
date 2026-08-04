# Design: Every machine in one fleet

Date: 2026-08-03
Status: APPROVED (design); implementation not started
Extends `docs/farcooler-design.md`. Follows
`docs/superpowers/specs/2026-08-02-worktree-management-design.md`, which named
this as its successor project.

## Problem

The "This Mac" dropdown is not a filter. `Preferences.remoteHost` is what the
app puts in `--host` on every CLI invocation
(`apps/macos/Sources/FarCooler/DaemonClient.swift:32`), so the Mac app talks to
exactly one daemon at a time. The sidebar is not showing you a filtered fleet;
it is showing you the only fleet it has.

That makes a remote agent something you have to go and look for. The product's
whole claim is that an agent blocked on a machine in another room is exactly as
urgent as one on this desk — and the app answers that claim by asking you to
switch machines to find out.

Worktree management made this worse in a useful way: now that every worktree on
a machine appears automatically, switching hosts throws away a sidebar full of
real rows and replaces it with a different one.

## What we are building

Every configured machine is connected at once. Every project appears in one
scroll area regardless of which machine it is on, with the machine named
alongside it. The picker goes away.

`workspace.host` is the routing key. That is not a new convention: the editor
feature already routes on `workspace.host ?? ""` with empty meaning this Mac
(`apps/macos/Sources/FarCooler/OpenInEditor.swift:19`), and the CLI already
stamps the field from the `--host` flag it was invoked with
(`crates/cli/src/main.rs`). This design adopts the convention the codebase has
already converged on rather than inventing one.

## Architecture

**`DaemonClient` becomes per-machine.** It is constructed with a target — the
empty string meaning this Mac — and derives its own `--host` arguments from
that, instead of reading a global preference. Everything else about it is
already right: it owns an event stream, a fleet slice, and an error.

**A new `FleetStore` owns the machines.** It holds `[String: DaemonClient]`
keyed by target, merges each client's workspaces into one list, and answers
`client(for: Workspace)`. Every mutation goes through it.

Its membership is always **the local machine plus one client per configured
host**: `Hosts.shared.all` lists remote machines only, and this Mac is the
implicit entry under the empty-string key. `FleetStore` observes `Hosts` so that
adding a machine in Settings brings up a client and removing one tears its
client down, without a relaunch.

Naming, because two things would otherwise both be "the fleet": `Fleet` stays
what it is today — the struct decoded from one machine's `workspace list --json`
— and `FleetStore` is the object holding N of them. `FleetStore` republishes
when any client's `@Published fleet` changes, so views observe one object rather
than subscribing to N.

**Failure isolation is structural.** One machine being unreachable is one object
in a bad state, not a flag threaded through shared code. This is the reason to
prefer a client per host over one client that takes a host parameter: there is
somewhere natural for "this machine is down, here is why, here is when we last
heard from it" to live.

**`ContentView.Selection` gains the host.** Short ids are the last eight hex
characters of a UUID minted per daemon. Across three machines and a hundred
worktrees the birthday collision probability is around one in a hundred
thousand, and the cost of losing that coin flip is acting on the wrong machine.
Carrying the host removes the class of bug rather than betting against it.

The per-pane plumbing is already in place: `hostArguments` is a parameter on
`TerminalPane`, `TerminalStream`, and `AgentStream`
(`apps/macos/Sources/FarCooler/TerminalPane.swift:18`) rather than a global
read. Those types do not change; they simply stop always being handed the same
value.

## Resilience

This is most of the work, and it starts below the app.

### The SSH layer

`crates/cli/src/remote.rs` sets `BatchMode`, `ControlMaster`, `ControlPath`, and
`ControlPersist`, and **no keepalives at all**. A peer that goes away silently —
a lid closes, a VPN drops, a server reboots — leaves ssh parked on a half-open
TCP connection indefinitely. The process never exits, so the app's `onEnd`
never fires, and the machine stops delivering events *while still looking
connected*. Nothing above this layer can compensate for it: there is no signal
to react to.

Add `ServerAliveInterval=15`, `ServerAliveCountMax=3`, `ConnectTimeout=10`.

**This lesson was already learned once.** `crates/client/src/ssh.rs:144-147`
sets `keepalive_interval: 30s` and `keepalive_max: 3`, with a comment that
begins "A phone sleeps and wakes; without keepalives a session that the…". The
iOS client speaks the protocol over russh and got this right. The Mac shells out
to `ssh` and never did.

The Mac uses 15s where the phone uses 30s. A phone pays for each probe in radio
wake-ups and battery; a Mac on mains power and usually stable networking does
not, and 45-second detection instead of 90 matters for a tool whose purpose is
noticing when an agent needs you.

`ControlMaster=auto` with a shared `ControlPath` means all sessions to a host
share one master, so the master dying takes them together. That is fine — they
reconnect together. A *stale* control socket is the hazard, and `ConnectTimeout`
is what bounds it.

### Reconnection

Per-host exponential backoff: 1s doubling to a 30s cap, with ±20% jitter so
several machines recovering from one network event do not retry in lockstep.

On reconnect, a full re-read. Anything that changed while the client was deaf is
only visible in a full read, which `onEnd` already does today.

**Wake and network transitions short-circuit the backoff.**
`NSWorkspace.didWakeNotification` and `NWPathMonitor` both reset every host's
backoff and retry immediately. Without this, opening a laptop lid means sitting
through a 30-second wait while everything looks broken — which is the single
most common way this feature will be experienced.

### Connection state

```
connecting
connected
reconnecting(attempt: Int)
unreachable(reason: String)
notInstalled
```

`notInstalled` is separate because it is not a failure to retry. It is a machine
that needs `host install`, and retrying it forever produces noise instead of the
one sentence that would fix it. `Hosts.probe` already distinguishes this
(`HostProbe.isInstalled`).

## The sidebar

The group key becomes `(host, project)`. Two machines can have a project of the
same name, so the key has to separate them even where the display looks alike.
The machine is shown as a secondary label on the project header, and only when
more than one machine is configured — on a fleet of one, saying which machine is
noise.

A state dot sits by the host label: absent when healthy, amber while
reconnecting, red when unreachable with the reason on hover. Reconnection is
otherwise silent. Clicking a red dot retries that machine immediately and resets
its backoff; Settings gets a "Reconnect all".

**An unreachable machine keeps its rows**, from the last good fetch, dimmed.
Reads work — a terminal opens and shows its last known screen. Mutations are
refused immediately, carrying the host's own error, rather than spawning a CLI
that will hang until `ConnectTimeout`. Nothing is queued: an action replayed
minutes later against changed state is a surprise nobody can attribute.

"Refused immediately" is `FleetStore`'s job, not each call site's: a mutation
routed to a client whose state is `unreachable` or `notInstalled` returns that
state's reason without spawning anything. A call site therefore cannot forget to
check, which is the point — there are around 138 of them.

A machine that has never connected has no last-good rows, so it contributes
none. It appears in the sidebar only as a header with its state dot, which is
how you find out it needs attention rather than wondering where it went.

## Adding a repository

The Add Repository sheet gains a machine picker, defaulting to this Mac.
Choosing a remote machine changes what the path field means, so the local file
browser is replaced by a typed path validated by that machine's daemon.

## What is deleted

- `apps/macos/Sources/FarCooler/MachinePicker.swift`
- `Preferences.remoteHost`
- `DaemonClient.hostChanged()`

`Hosts.shared` stays. It is the list of configured machines, which is what it
always actually was; only the *selection* it carried is going away.

## Testing

A Rust test asserting `ssh_args` carries `ServerAliveInterval`,
`ServerAliveCountMax`, and `ConnectTimeout`. The whole resilience story rests on
those three options and their absence is invisible until a connection dies — the
existing test in that file already asserts `BatchMode` for the same reason.

Swift has no test infrastructure here, so reconnection is a manual checklist,
run per case:

| Case | Expected |
|---|---|
| `farcooler daemon stop` on a remote host | amber within ~1s, reconnects when restarted |
| Drop the network (Wi-Fi off) | amber, then red after retries; rows stay, dimmed |
| Sleep and wake the Mac | reconnects immediately, not after a backoff wait |
| Reboot a remote host | red while down, reconnects unaided |
| Unreachable host, attempt a mutation | refused at once with that host's error, no hang |
| A machine with no daemon installed | `notInstalled`, not a retry loop |

## Phase two: iOS

Deferred, and in this spec so the design accounts for it rather than being
surprised by it.

**iOS is already further along architecturally.** `Connection.swift` is
documented as "One host's session and the state a view renders from it" — the
per-host unit this design is introducing on the Mac already exists there. It
speaks the protocol over russh through a Rust FFI core rather than shelling out
to a CLI, and it already has keepalives.

What iOS shares with today's Mac is the *selection*: `HostStore` holds
`hosts: [Host]` plus a `selected`, and `RootView` keys its whole view tree on
the selected host so switching machines rebuilds everything below. That is the
same one-machine-at-a-time model, presented differently.

So phase two is the same change with a different shape: N live `Connection`
objects instead of one selected, merged into one list, with `FleetView` and
`WorkspaceListView` grouping by machine.

Two things are genuinely different and must not be assumed away:

- **Background execution.** iOS suspends the app; N held SSH sessions will be
  torn down on backgrounding and must all re-establish on foreground. The Mac's
  wake handling is the analogue but the constraint is far harsher.
- **Cellular.** N concurrent sessions with 30-second keepalives is a materially
  different battery and data proposition than one. The phone may well want a
  connect-on-demand policy where the Mac holds all of them — the same product
  answer is not automatically right on both.

Phase two gets its own brainstorm before implementation. This spec commits only
to the macOS half.
