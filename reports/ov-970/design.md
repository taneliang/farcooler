# ov-970: one builder on several tickets

From the owner's answer to ov-964 item 5: "You can put as many tickets as appropriate on one builder... the
orchestrator should be able to make this decision. E.g. maybe we'll want to have a 'small fixes' PR." That is
option 5B: one run for several tickets, a lead with the rest following it. Stacked on ov-968.

## Where it stands

- `agent dispatch` on several tracked tickets sends one `Start` per ticket: one run, one worktree, one agent and
  one branch each (`plan_agent_start.rs`). A `--new` worktree is refused with more than one ticket.
- Each run owns exactly one ticket's card, and the projection moves only that card.

## Design

1. **Start takes followers.** `Command::Start { with }`, the other tickets. Each must be a ticket at rest, not
   already following and not leading. At most 32. The lead runs as today. Each follower records `follows:
   lead`, and the lead records `followers`. Nothing else about a follower changes: it holds its own resource
   and no run.
2. **The followers' cards follow the lead.** The projection places a follower where its lead is: the same
   column and line stage. While it follows, it isn't at rest, so the orchestrator's line writes are refused
   on it as they are on the lead. `Move`, `Start`, `SendBack` and another `Start --with` naming it are refused
   (`Phase`).
3. **They close with it.** When the lead ends `Done` or `Cancelled`, each follower ends the same way in the
   same step and stops following. A failed lead leaves them following, and the board shows them as needing
   a decision, like the lead. A follower that closed can be reopened on its own (ov-968).
4. **One agent, one branch, one pull request.** The followers ride on every effect (`Effect.with`), so the
   agent's brief names all the tickets: "You are working on tickets ov-1, ov-2 and ov-3." The run's tree,
   branch and pull request are the lead's, as for any run.
5. **Wire and CLI.** `Action::Start { with }`. `workflow start ... --with KEY` (repeat). `agent dispatch` with
   several tracked tickets on one builder now starts **one** run, the first ticket leading, so `--new/--branch`
   are allowed there. The orchestrator decides how many by what it dispatches.
6. **Store.** A Start's fence on open subagent records and blocks covers every follower too.

## Test

Reducer: start with two followers; they follow the lead's place and refuse moves; landing the lead closes all
three; cancelling the lead cancels them; a failed lead leaves them following; invalid followers refused.
Store: the projection moves all three cards with the run. Daemon, over the socket: one Start on three tickets
makes one worktree and branch, the build's brief names all three, and the three cards end Done. CLI: the
dispatch of three tickets sends one Start with `with`.
