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

## Fix round: the owner's grant (review: pane descent admits everything under the pane)

Pane descent alone admits the orchestrator's shell commands, subagents and
scripts. So the live Prepare also needs the owner's word, once.

- **The grant** (`crates/daemon/src/cutover_grant.rs`). An answer
  `Allow cutover once` (exact, trimmed, any case) to a question on the same
  card that offered it, on a card of the orchestrator's own workspace, actor
  `user`, `said_from` `app` or `device`, at most 24 hours old, not superseded
  and not spent. The live Prepare spends it (a `runner` note on the card naming
  the answer and the attempt) under one lock, after every other check, before
  the hold. Recovery steps ride the attempt the Prepare pinned.
- **Where the seat is recorded.** The note's `actor`: an answer claiming `user`
  is walked (`task_ops::answer_actor`, ov-630), so one from a pane is
  `manager`/`agent:…`/`unknown` whatever `--actor` says. The walk is a
  guardrail (an orphan with a terminal of its own reads as a person), so the
  grant doesn't rest on it: `task.note` also stamps `extra.said_from`, the
  daemon's word, each from an authenticated seat (orchestrator, from the
  ov-971 review):
  - `app`: the caller process satisfies the designated code requirement of the
    bundle this daemon ships in (`code_identity`, Security.framework by pid,
    start time rechecked). The live Canary's DR pins
    `identifier "com.farcooler.FarCooler.canary"` and team `H6A2TRW47J`; its
    running app process passes it and its bundled CLI fails it (checked with
    `codesign --verify -R`, read-only). The app talks to `farcoolerd.sock`
    from its own process (lsof), so the daemon sees the app's pid.
  - `device`: the answer carries a step-up (ov-856) verified against the
    owner's phone's trusted key, over this very request. `task.note` takes an
    optional step-up (`step_up::OPTIONAL`) and never needs one. A browser's
    passkey and a member's key don't count. A device line alone is a preamble
    any local process can write, and an ssh session one any process of the
    owner's can open, so neither counts.
  - `cli`: anything else, the CLI in any terminal included.
  A caller sending `said_from` is refused, as with `message`.
- **Recheck in recovery.** Nothing but recovery can write the database while it
  holds it, so a lead, role or login can't change there; the recheck guards an
  edited or restored file. Each recovery step, `cutover_delegate::admits` reads
  the database read-only (`engine_cutover::startup::owners_orchestrator`): the
  terminal is still the workspace's orchestrator, with no member login, no rep,
  and no member leading the workspace (the owner's case of
  `refuse_another_leads_login`).
- **Journal.** Commit's note names the grant's answer id. `record_caller`
  already fsyncs the entry and its directory (`closing.rs:58-59`).
- **Not built here.** iOS and Android don't sign an answer yet, so until a
  follow-up ships that, the owner grants from the Mac app. The watch can't
  sign either.

Tests, each red with its check removed: `cutover_grant_tests.rs` (no grant,
agent-written, `--actor`, a forged stamp, a terminal's, old, spent, unasked,
another board; the app's grants once, the phone's signed answer grants and
its unsigned one doesn't), `code_identity` (a process is the program its
signature says),
`answer_seat_tests::no_answer_from_the_cli_grants_a_cutover` (real processes),
`recovery_tests::live_prepare_refuses_the_owners_orchestrator_without_the_owners_grant`,
`cutover_delegate_tests::every_recovery_step_rechecks_role_login_and_lead`, and
the e2e, now with the owner's grant.
