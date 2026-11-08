# ov-974: the owner's orchestrator can run the cutover

## Where the brief is right, and where it isn't

- `recovery_seat` (`crates/daemon/src/caller_seat.rs:148`) and the recovery
  handler (`rpc/recovery.rs:34`, `caller_is_a_person`) admit only `Seat::Person`. Right.
- The brief misses the first gate: the **live** daemon's Prepare
  (`rpc.rs:799-805`) captures its admission with `Admission::HOST_WIDE` and calls
  `require_person()`. No orchestrator gets past it, so the owner's orchestrator
  could never reach recovery at all. It changes too.
- "Check the pane table is readable then": it isn't, usefully. A recovery
  process has no Store, no inventory and no registry (`rpc/recovery.rs:1-8`),
  and it must not hold the database open: every recovery op takes
  `JoinedOwners::acquire`, the kernel proof that nobody else has the file open.
  Opening the Store per request to read `terminals`, logins and leads would race
  that proof, and between Prepare and Commit the file is frozen mid-transfer.

## Design

**Decide once, where the rules live; pin the process; check the pin in recovery.**

1. **Live Prepare** (has the Store). The caller's seat is walked as now. A
   person takes the existing path. An agent in a pane is admitted with
   `Admission::capture` for *that pane's workspace*, which is exactly the rule
   `workflow.move` uses (ov-951): the pane is that workspace's `Orchestrator`,
   `refuse_another_leads_login` passes, and `caller_person` is
   `Person::Owner` (so a member's orchestrator is refused). Not reused by copy:
   the same function is called.
2. Before the hold mark is written, the live daemon writes
   `<root>/cutover-delegate.json`: the attempt, the terminal and workspace ids,
   and the **pane's root process: pid and start time**
   (`process_incarnation`). A person's Prepare removes any such file, so an
   earlier attempt's delegate never outlives a Prepare.
3. **Recovery** admits a caller when it is a person (as today), **or** when the
   delegate file names the attempt now in hand (the hold mark's, or the
   baseline's once Prepare has run) and the walk from the caller, up its
   parents, reaches the named pane root with the same start time before it
   meets any tmux server. The connect-time pid + start check stays.
4. **Commit** records the caller as "the owner's orchestrator (terminal …,
   workspace …), acting for the owner: pid …".
5. Finish and Recover, once they succeed, remove the delegate.

### Why this is not a new way in

- Only the live daemon's full admission ever names a pane; recovery only asks
  "is this caller inside that one process tree, still the same process". A
  builder pane, a member's orchestrator and another lead's pane are refused at
  Prepare and so never named; in recovery any other pane fails the walk.
- An orphan the orchestrator left behind (`(cmd &)`, reparented to pid 1)
  doesn't reach the pane root: refused, as today.
- A pane restarted between Prepare and recovery has a new root process: refused.
  The owner can still run the rest by hand.
- The delegate is a file beside the database, as trusted as the hold mark and
  the database themselves: a process that can write it can already write the
  database it guards. The seat walk was always a guardrail against an agent
  *asking*, not a boundary against one writing files (`caller_seat.rs` docs).

## Tests (red first)

- e2e, real socket, scratch runner (`tests/the_owners_orchestrator_runs_a_cutover.rs`):
  a real tmux pane, the workspace's orchestrator on the runner's login, runs
  Prepare on the live daemon (exit 75), then Prepare, Commit, Verify, Release,
  Finish in recovery from the same pane shell; the journal names the
  orchestrator at Commit.
- Live Prepare refuses a builder pane, a member's orchestrator, and an
  orchestrator on another lead's login; each writes no hold and no delegate.
- Recovery refuses a pane not named by the delegate, a delegate for another
  attempt, and a pane root with a different start time.

## Skill

`crates/daemon/assets/manager/SKILL.md` "Cutover": the owner's orchestrator may
run it for the owner, from its own pane; no other agent ever does.
