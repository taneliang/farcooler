# ov-886: restart hooks keep to their own launch

Base: main `4dac77977`. This replaces the source-only drafts on `ef000e9`…`069da6d15` as the design being built. Those reports stay as history; what they established is used below and cited where it matters.

## Does the card still apply?

Yes, in a narrower form than when it was written.

- **The agy guard isn't on main.** It lives on the ov-878 branch (`59b571f3c`), and ov-878 is in backlog waiting on this card. There is no agy harness or guard on main to preserve. ov-878 must keep its guard when it lands, unless it also adopts the launch fence below and proves the agy producer carries it.
- **The engine cutover didn't touch interactive restart.** Engine builds run in a private tmux server with their own socket, one per binding (`workflow_runtime/coding_native.rs:68,224-231`), and are never respawned. Interactive panes still restart through `Service::relaunch_held` (`service.rs:4300`), which respawns the pane in place (`respawn-pane -k`) on the same session when there is a transcript to reopen.
- **Ingress still can't tell two runs of one pane apart.** `HookIngress::terminal_for` (`hook_ingress.rs:243`) routes by the session a row claims, then by Claude's registry (`registry_binding.rs:34`), then by the only unbound pane of that agent in the worktree (`hook_ingress.rs:324`). A hook carries the agent, the event and the producer's payload, and nothing about which launch fired it (`agent-hooks/src/wire.rs:17`).

So today, a hook fired by the process a restart killed is indistinguishable from one fired by its replacement:

| Harness | Restart behavior today | Where a late hook from the killed process lands |
| --- | --- | --- |
| Claude, resumable | `claude --resume S`, same session `S` | The pane, by `S`. Its `Stop` can end the new process's turn, settle the new process's held ask, or be folded into the new transcript. |
| Claude, nothing to resume | fresh `claude`, new session `S2`; the row still says `S1` until the registry rebinds it | The pane, by `S1`, until the rebind. After it, by the registry, if the dead process's registry entry still matches the pane's tty. |
| Codex, Cursor | rows never name a session; resume or fresh | The pane, by worktree match. Indefinitely. |

This is the same pane's previous run, not another pane's conversation. That makes it a correctness bug and a held-ask hazard (an ask from a dead process offered to a phone, a verdict written to nothing). It becomes cross-session leakage as soon as a restart clears or replaces a binding, which is what ov-878's agy restart needs to do.

## What identifies a launch

**A launch id: a fresh UUID minted by the daemon for every launch of an agent into a pane, exported to the pane as `FARCOOLER_LAUNCH`, and copied by the hook CLI into its frame.**

- **Its value is minted by the daemon, not by the producer.** The producer's payload isn't touched; the field is on our own envelope (`HookLine.launch`), written by our own CLI. It's an optional field, ignored by a daemon that predates it, and absent from a frame sent by a CLI that predates it.
- **It travels the way the engine's socket already travels.** The engine's native builds depend on an env var set at launch reaching the hook CLI through the agent: `FARCOOLER_NATIVE_HOOK_SOCKET` (`cli/src/hook.rs:96-107`). `FARCOOLER_ACTOR`, exported the same way by `with_pane_env`, demonstrably survives the login shell into Claude's own children: this pane's board notes are filed as `agent:01a1be05-…`, and `echo $FARCOOLER_ACTOR` from this session's tool shell prints it. A Claude hook is a child of the same process with the same environment.
- **It isn't proven for Codex or Cursor.** The ov-886 source traces (`fed5b102f`, `069da6d15`) found Codex 0.166 replays its hook host's environment into hooks, and that Far Cooler's current `-c check_for_update_on_startup=false` keeps the TUI on an embedded host, but neither was observed at runtime. The design is shaped so that an unproven harness is **never worse off than today**: see "untagged frames" below.
- **It isn't authentication.** Anything running as this user can read or copy an environment variable, and an agent's own children inherit it. It's a correlation value that lets the daemon tell an ordinary late hook from a fresh one. The threat it answers is a delayed or orphaned hook from a killed run, not a hostile process running as the same user (which can already write to the ingress socket today).

## Where the current launch is recorded

Rows in the existing `meta` table, keyed `pane_launch/<terminal id>`, owned by a new store module `pane_launches.rs`.

- **No migration, on purpose.** A new table would move the schema to 69, and `engine_cutover::preflight` (`store/src/engine_cutover.rs:217`) accepts exactly 60 or 65–68, with pinned catalog hashes. Any runner that hasn't cut over yet would be refused. `meta` already holds per-device rows (`device_added_by.rs`), and the cutover conserves its rows like any other table's.
- **Durable.** A daemon restart reads it straight back; there is no in-memory epoch to lose.
- **One row per pane**, replaced on each launch. The previous launch is retired the moment it's replaced, and nothing remembers retired launches: a launch id that no row holds is retired by definition, so no tombstone list grows and none needs collecting.
- **No cascade from `terminals`.** `delete_terminal` removes the pane's row. A pane deleted by cascade (its worktree or workspace going) leaves its row behind, and every lookup checks the pane still exists, so that row routes nothing. The leftover is one short row per such pane.
- **An older build never reads these keys.** It launches without exporting a launch id, so its frames are untagged, and the row left behind names a launch that nothing carries.

Store operations, each a single transaction:

- `begin_launch(terminal, expected_version) -> (Terminal, Launch { id, prior })` — compare-and-set on the terminal's `resource_version`, bump it, and write a fresh launch id in place of the prior one. Two restarts racing each other can't both succeed: the loser gets `ResourceConflict` before it spawns anything.
- `roll_back_launch(terminal, attempted, prior)` — put `prior` back **only if** `attempted` is still the current launch. If a newer launch has replaced it, nothing changes. Removing the row is the rollback when there was no prior.
- `terminal_for_launch(launch) -> Option<Uuid>`.

## Launch, restart, rollback

Every launch of an agent into a pane (`create_terminal`, `split_terminal`, `relaunch_held`) does:

1. **`begin_launch`**, then build the command with `FARCOOLER_LAUNCH=<new>` beside `FARCOOLER_ACTOR`. From this instant the old launch is retired: its hooks reach nothing. (The orchestrator seat is taken before this, as it already was.)
2. **Cancel the pane's in-flight hook state** (`HookIngress::relaunched`): the held ask (settled as unanswered), the half-assembled message and the transcript tail. This is the cancel half of "cancel or drain" for anything the old launch had in flight. It's narrower than `forget`, which is for a deleted pane: the pane's conversation projection and subagent rows carry on into the new launch.
3. **Pre-spawn checks** (`refuse_a_real_agent`, `still_on_its_login`), which need the finished command, **then spawn.**
4. **If either fails**, `roll_back_launch(attempted, prior)`. The old launch is current again only if no newer launch replaced this one. This is safe even when `respawn-pane -k` killed the old process before failing: restoring `prior` only lets the old process's own late hooks back into its own pane, which is where things stood before the restart was asked for. It never overwrites a newer launch.

Only agent presets get a launch id. A shell, a changes pane or a web pane has no hooks to fence.

## Ingress

### Tagged frames (carrying a launch id)

A tagged frame is routed **only** by its launch:

- **Launch held by no row** — a retired launch, or one from another runner. Dropped before anything else happens: no fence bookkeeping, no ask, no followup, no event. A frame that waits for an answer gets the empty verdict at once, so the agent doesn't sit out its 400 ms.
- **Launch held by a row** — that pane, provided:
  - the row's preset runs the agent the frame says it is (a `cursor-agent` someone starts inside a Claude pane inherits the Claude pane's launch id, and isn't that pane's agent);
  - the pane isn't still a chat;
  - for **Claude**, the frame's session is the one the row names, or Claude's registry proves the new session is in this pane (`registry_binding::bind_pane`, unchanged, which also covers `/clear` and a clean restart that minted a fresh session). A `claude` started by hand from inside an agent pane inherits that pane's launch id; without this check its hooks would be pulled into the pane;
  - for **Codex and Cursor**, the frame's `cwd` is the pane's worktree. Both report their launch directory as `cwd` (`hook_ingress.rs:461-466`), so a mismatch means the id was carried somewhere it doesn't belong.

If a check fails, the frame is dropped. It doesn't fall back to session or worktree matching: a frame that says which launch it came from is judged on that.

### Untagged frames

Routed exactly as today. That covers CLIs that predate the field, harnesses whose producer strips the environment, panes launched by an older daemon, and agents nobody launched from Far Cooler. If an unproven harness turns out not to carry the variable, its panes behave as they do now; they don't stop reporting.

### Making the check and its effects atomic

The routing check and the effects it permits must not be separated by a restart. A frame could otherwise be admitted while its launch is current and act after step 3 has cleared the pane.

- `HookIngress` gets a **launch gate**: 64 `std::sync::Mutex<()>` stripes, picked by pane, so a slow hook on one pane holds up no other pane's.
- A tagged frame's **synchronous effects** run inside its pane's stripe, after re-reading the pane's launch inside it: taking a queued followup, holding an ask, starting a transcript tail, folding the frame into events and handing those to the sink, and each transcript-tail delivery. If the launch moved, they don't run.
- `begin_launch` commits before the stripe is taken for step 2. So either a frame's whole effect block finished before step 2, and step 2 cancels what it left (tail, ask, assembler), or the block starts after the commit and sees the new launch.
- **Untagged frames take no gate.** They're routed as on main, and a hook that predates launches mustn't start waiting on one.
- **Nothing slow runs under it.** Its store reads are single SQLite reads; it's never held across an `.await`. `observe_cwd` can wait on the claims ledger, so it runs after the stripe is released. A stale frame that passed the gate just before a relaunch can therefore still report the worktree it was working in, for its own pane's workspace; that's a claim, not an event or an ask, and the same report the process would have made a moment earlier.

What stays outside the gate, on purpose:

- **Bytes already written to a hook's socket.** A followup taken inside the gate and written to an old process just before it is killed is lost with that process, as a message typed into a pane being restarted is lost today. The daemon can't recall bytes from a socket; it can only stop deciding new things for a retired launch, which the gate does.
- **A held ask's verdict.** `forget` settles it as unanswered, which writes the empty verdict, so the old process (if it's still alive) defers to its keyboard.

## Concurrency, daemon restart, duplicates

- **Two restarts at once**: `begin_launch` is a version CAS, so one wins; the other errors before spawning.
- **A restart and a create**: different terminals, different rows; launch ids are unique.
- **Daemon restart**: the table is durable; a frame from a launch the table doesn't hold is dropped whether the daemon was restarted or not. A restart interrupted between `begin_launch` and the spawn leaves the new launch current with nothing running it. The pane's hooks are fenced until its next restart, and the old process, if it survived, is fenced too. Never a wrong attachment.
- **Two rows claiming one session**: unchanged for untagged frames (refused). A tagged frame names one row by launch, so it can't be ambiguous.
- **Pane deleted**: `delete_terminal` removes its launch row; a row left by a cascade delete names a pane that no longer exists, and lookups refuse it. Either way its hooks reach nothing.

## What this doesn't do

- **No proof that Codex or Cursor hooks carry the variable at runtime.** Claude's is evidenced as above. For the others, untagged frames keep today's behavior, so the fence applies wherever the variable arrives and nothing regresses where it doesn't. Proving Codex and Cursor needs a real run, which this lane isn't cleared to do (Claude-only until Nov 10; no provider probes).
- **Unmanaged agents in a managed worktree** still attach by worktree match through the untagged path, as they do today. Closing that (refusing untagged frames for a pane whose current launch has already been heard tagged) is a follow-up once Codex and Cursor are proven, and it's noted on the card.
- **agy** isn't on main. ov-878 can adopt this fence by exporting the launch id on its launch path and showing an agy hook carries it; until then its guard stays.

## Regressions (each seen red first)

| Test | Red today because |
| --- | --- |
| store: two `begin_launch` on one version, only one wins; rollback restores only the launch it replaced; a newer launch survives a stale rollback; a deleted pane's launch goes with it | no launch table |
| ingress, per harness (Claude, Codex, Cursor): a late frame from the retired launch reaches nothing, including Claude resuming the same session | routed to the pane by session or worktree |
| ingress, per harness: a frame from the current launch reaches the pane; for Codex, even with a second Codex pane in the same worktree | ambiguous worktree match refuses both |
| ingress: a Claude started by hand inside an agent pane (inherited launch id, a session the row doesn't name, no registry proof) reaches nothing | — (guards the new path against pulling it in) |
| ingress: a frame admitted, then the launch replaced and the pane forgotten, then its effects run: nothing is emitted, no tail starts | effects aren't re-checked |
| ingress: a retired launch's waiting frame gets the empty verdict and no ask is held | the ask is held on the pane |
| service: a failed spawn rolls the launch back; a newer launch that won in between survives it | no rollback |
| service: an agent launch exports `FARCOOLER_LAUNCH`; a shell doesn't | not exported |
| cli: the frame carries the pane's launch id; a value that isn't a UUID is dropped | no field |
