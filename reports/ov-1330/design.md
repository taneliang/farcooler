# ov-1330: quiesce without hunting processes

## The handle

The native agent no longer runs as a child of a shell that the daemon finds later by tty. tmux runs
`farcoolerd --native-hold <socket> -- /bin/sh -c '<launch>'` in the pane. That process is the **hold**:

1. It binds `<socket>`, a Unix socket under the runner's private root, before anything else runs.
2. It spawns the agent as its own child, in a new process group led by that child (`setpgid(0, 0)`
   in the child before `exec`), and makes that group the terminal's foreground group.
3. It never reaps the child until the group is proved empty.

The ownership handle is the pair (hold socket, unreaped child). The daemon holds the socket path from the
moment it opens the pane: it waits for the hold's `ready` before it writes the owner record or releases the
launch barrier. The hold holds the child from `fork`.

Every signal the engine sends a native agent is now `killpg(leader, sig)`, sent by the hold, where `leader` is
the pid `fork` returned to it. The daemon sends no signals to native agents at all; it asks the hold, over the
socket, to `stop`, `cont` or `end`.

## Why no PID or tty reuse can redirect a signal

* **The group id cannot be recycled while the hold may signal it.** A pid is not reused while its process is
  unreaped: the child's pid stays allocated, as a zombie if it has exited, until its parent (the hold) waits
  for it. And a pid in use as a process group id is not handed out either. Both kernels guarantee this: Linux
  frees a pid number only when no task is attached to it in any role (pid, pgid or sid), and XNU's pid
  allocator skips any number in use as a pid, pgid or session id. So from `fork` until the hold reaps the
  child, `leader` names this group and no other.
* **The hold reaps only after its last signal.** Reaping and signalling are both done under one lock, and
  `signal_group` refuses once the child is reaped. There is no window between a check and the signal: the
  kernel resolves the group in the same `killpg` call.
* **Nothing is found by looking.** No signal's target comes from a process listing, a tty name or a pid that
  tmux reported. The hold reads the process table only to wait (are any members of group `leader` still
  alive, are they all stopped); while `leader` is pinned, a row with that pgid is one of ours, and a stale or
  torn read can only make it wait longer.
* **The daemon's side cannot be redirected either.** It talks to a socket path in its own 0700 root, derived
  from the binding. If the hold is gone, the connect fails; nothing is signalled in its place.

macOS and Linux are the same design: `fork`, `setpgid`, `tcsetpgrp`, `killpg` and `waitid(WNOWAIT)` are
POSIX. The one platform difference is how `fork` races a group-wide `SIGKILL`. Linux makes `fork` fail and
restart when a group signal arrives during it; XNU can let a child join the group just after the signal. The
hold therefore repeats `killpg(leader, SIGKILL)` until two consecutive reads of the table show no member but
the (zombie) leader. A child created during `fork` exists before its parent finishes `fork`, so a read that
misses the child still sees the parent.

## Proof of quiescence

`end` is: `SIGCONT` and `SIGTERM` to the group (an agent can then end the work it alone holds, see below),
up to one second for it to empty, then `SIGKILL` until it is empty; then reap the leader, unlink the socket,
answer `ended`, and exit with the agent's status. The hold does the same when the agent exits by itself, and
on `SIGHUP`/`SIGTERM` (tmux closing the pane).

So for a hold-launched owner, quiescence is proved by the hold's `ended` answer, or by its socket being gone,
because the hold unlinks it only after the group is empty. The daemon never reads the tty for that proof, so
a tty that a later pane reuses can neither be signalled nor make a finished owner look busy.

If the socket is there but nobody answers (the hold was killed), the daemon falls back to the observation it
already had for a vanished owner: the pane is dead or gone and nothing is on its tty. It signals nothing. An
owner launched by an earlier engine (no hold) gets the same observation-only treatment.

## What the group does not cover

A process that leaves the agent's process group (`setsid`, or `setpgid` into a group of its own) is outside
the hold's authority. Codex runs its commands that way. The previous design reached them by parent pid, which
is exactly the kind of proof six reviews broke. Now:

* the hold's `SIGTERM` stage gives the agent a second to end them itself; the agent is their parent, so its
  own `kill` of them is race-free;
* Codex sets `PR_SET_PDEATHSIG` on Linux, so they end when it does;
* on macOS, a command Codex started and did not end runs to completion.

That is a narrower stop than before, and it is the price of never signalling a process the engine did not
start. It is recorded here rather than papered over.

## What changed

* `crates/daemon/src/native_hold.rs`: the hold (`serve`) and its client (`stop`, `cont`, `end`).
* `quiesce_native` asks the hold to end; the tty and descendant walk is gone from it.
* The orchestrator's pause (`native_control::freeze`) and a landing gate's pause (`yielding::Target::Hold`,
  replacing `Target::Tty`) also go through the hold.
* A source test fails the build if `coding_native.rs`, `native_control.rs`, `native_hold.rs` or `yielding.rs`
  signal anything other than through `native_hold::signal_group`, or if a tty walk returns to `yielding.rs`.
