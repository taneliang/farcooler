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
up to one second for it to empty, then `SIGKILL` until it is empty; then wait for every **descendant** of the hold
to be gone (next section); then reap the leader, unlink the socket, answer `ended`, and exit with the agent's
status. The hold does the same when the agent exits by itself, and on `SIGHUP`/`SIGTERM` (tmux closing the pane).

So for a hold-launched owner, quiescence is proved by the hold's `ended` answer, or by its socket being gone,
because the hold unlinks it only after the group is empty and no descendant is alive. The daemon never reads the tty
for that proof, so a tty that a later pane reuses can neither be signalled nor make a finished owner look busy.

If the socket is there but nobody answers (the hold was killed), the daemon falls back to the observation it
already had for a vanished owner: the pane is dead or gone and nothing is on its tty. It signals nothing. An
owner launched by an earlier engine (no hold) gets the same observation-only treatment.

## What leaves the group: followed, never signalled (fix round 1)

A process that leaves the agent's process group (`setsid`, or `setpgid` into a group of its own) is outside the
hold's authority, and the hold never signals it: that would be signalling a pid found by looking. Codex runs its
commands that way. Round 0 let such a process run on after `ended`; the review rightly called that settling over
work that may still be writing the worktree. Now the hold **follows** every descendant, in the group or not, and
refuses to say `ended` while one is alive:

* **Linux: the hold is a child subreaper** (`prctl(PR_SET_CHILD_SUBREAPER)` before it forks). An orphaned
  descendant is reparented to the hold, never to init, so every live descendant has a chain of live parents back to
  the hold's pid, which is alive. One read of `/proc` finds them all; nothing has to be remembered.
* **macOS: the kernel's unique ids.** XNU gives every process a 64-bit `p_uniqueid` that is never reused, and
  records the unique id of the process that forked it (`p_puniqueid`, `PROC_PIDUNIQIDENTIFIERINFO`). That record is
  **kept when the process is reparented to launchd**: measured on this Mac, a grandchild that called `setsid` after
  its parent exited showed `ppid=1` and still `puniqueid` = its dead parent's unique id. The hold learns the unique
  ids of its descendants (seeded with its own) every 100 ms while the agent runs and on every look during `end`; an id
  once learned is ours for good, because it is never reused.

A look only waits. Nothing it finds is signalled, and a look that misses a process can only see less of the
lineage, never take someone else's process for ours: an identity is learned only from its parent's.

Then `end`, once the group is empty, waits up to five seconds for the descendants outside it. If the agent ended
them (Codex kills its commands on `SIGTERM`; it is their parent, so that is race-free, and on Linux they also have
`PR_SET_PDEATHSIG`), the hold answers `ended`. If one is still alive, the hold answers **`unsettled`**, keeps its
socket and its unreaped leader, and stays; the daemon maps `unsettled` to a refusal ("native agent left a process
running outside its group"), so the step is not settled. A later `end` looks again; when the last descendant
exits, the hold answers `ended` and exits. The same holds when the agent exits by itself or tmux hangs the pane up:
the hold does not remove its socket or exit while a descendant lives, so a missing socket still means "nothing of
this agent's is running".

**The residual, on macOS only.** The hold learns a unique id only by seeing the process alive. A descendant B that
forks C and exits within one 100 ms learning interval, before the hold ever saw B, leaves C with a parent id the hold
never learned (C reparented to launchd). Such a C is invisible, as it is to every unprivileged observer on macOS
(the complete trackers, coalitions and Endpoint Security, need privilege). Linux has no residual: the subreaper sees
C whatever B did. This is narrower than round 0 (which ignored every process outside the group) and is recorded
rather than claimed away.

`stop` (a landing gate's pause, the orchestrator's freeze) still stops only the group: a descendant outside it keeps
running during a pause. That costs CPU during a gate, not correctness.

## Ordering of control requests (fix round 1)

Round 0 answered each connection on its own thread. A landing gate *tells* `stop` and later `cont` without waiting,
so a slow `stop` thread could take the lock after the `cont` and leave the builder stopped with no gate left to
continue it. The hold now answers on its accept thread, one request at a time, in the order clients connected
(the kernel queues a listening Unix socket's connections first in, first out on both systems). The daemon tells
under its pause lock, so connection order is decision order. A client that connects and never writes delays the
queue by at most the five-second read timeout; only the runner's own user can connect.

## What changed

* `crates/daemon/src/native_hold.rs`: the hold (`serve`) and its client (`stop`, `cont`, `end`).
* `quiesce_native` asks the hold to end; the tty and descendant walk is gone from it.
* The orchestrator's pause (`native_control::freeze`) and a landing gate's pause (`yielding::Target::Hold`,
  replacing `Target::Tty`) also go through the hold.
* `native_hold::tests::the_native_engine_signals_only_its_own_group` reads the shipped (non-test) source:
  `coding_native.rs` and `native_control.rs` may make no signal call at all; `native_hold.rs` may make exactly
  one, `libc::killpg(held.leader.0, …)` inside `signal_group`; `yielding.rs` may not read the process table or
  walk a tty (its `Target::Group` signalling of engine-created command groups is outside this ticket).
  `the_signal_check_catches_what_it_forbids` shows each rule failing, and planting
  `for pid in snapshot()?.under_tty(..) { libc::kill(pid, SIGSTOP) }` in `native_control.rs` turned the real
  check red before it was removed.
* Owner records gain `hold: true` (absent, so false, in records an earlier engine wrote). An earlier engine
  cannot read a record with the new field: rolling back with a native step in flight leaves that step Uncertain.

## Verification (fix round 1, after both review findings were fixed; macOS)

All foreground, at most four build jobs and four test threads, no gate, no formatter.

* The three ov-1319 tests, `--stress-count 30 --test-threads 4 --retries 0`: **30/30 iterations, 90 passes,
  0 failures** in 254.7 s (`stress.txt`). (A first attempt failed only because `farcooler-cli`, which the steering
  test needs, was not built in this worktree's target; it was built and the whole stress rerun.)
* The daemon crate, `cargo nextest run -p farcooler-daemon --test-threads 4 --retries 0 --no-fail-fast`, run as
  two partitions to stay in the foreground: **1,965 + 1,921 = 3,886 passed, 0 failed** (`daemon-1.txt`,
  `daemon-2.txt`). No test failed, so there is no flake to record.
* `cargo clippy -p farcooler-daemon --all-targets -- -D warnings`, `file-size-budget.py`,
  `macos-rust-tests.py --self-test --check`, `doc-comment-check.py`, `swallow-lint.py`, `test-window-lint.py`
  and `copy-lint.py` pass.
* The new regressions fail without their fixes. Restoring a thread per connection, and counting no strays in a
  look, turned all three red (`a_stop_asked_first_is_done_first_even_when_its_words_arrive_last`,
  `a_process_that_left_the_group_keeps_end_from_succeeding`,
  `a_stray_whose_parent_exited_first_is_still_accounted_for`) before the change was reverted.

Not run here: Linux. The Linux paths (`PR_SET_CHILD_SUBREAPER`, the `/proc` walk) are exercised by the same tests in
CI's Linux job.

New tests: `native_hold::tests` (stop, continue and end a group with a background job; an agent that exits takes
its group with it and the hold exits with its status; a reaped leader is never signalled even when its number
is a live process; the source check and its self-test; fix round 1: a stop asked first is done first though its
words arrive after a later continue's; a `setsid` command that ignores SIGTERM and keeps writing makes `end` answer
`unsettled`, keeps the socket and stays alive, never signalled, and `end` answers `ended` once it is gone; the
same for such a command whose parent exited before `end`; an agent that kills its own stray on SIGTERM ends
cleanly) and `quiesce_order_tests` (a live owner's whole group is ended; an owner that exited first, or exits while
being ended, is quiescent; an agent ignoring SIGTERM is killed after the grace; a tty a foreign session holds is
never signalled, whether the hold answers or was killed).
