# ov-968: send a ticket back, reopen it, adopt its branch

From the owner's answer to ov-964 item 3: adopt an existing branch, one send-back command with a reason the
agent receives, and the orchestrator can reopen a closed ticket. Stacked on ov-967 (named worktrees).

## Where it stands

- A run goes back to its build step only from a failed review (`Review` with a reason, at most two times in
  schema 1, or a schema 2 back edge). The reason is recorded in `rework_history`, but the build effect doesn't
  carry it and `briefed` (`coding_execute.rs`) doesn't say it, so **the agent never sees it**.
- A ticket that closes (`Done`, `Cancelled`) leaves `items` for `archive` on the next step. `Enqueue` and
  `Track` refuse an archived name, so a closed card can't come back under the engine at all.
- Start with `--worktree/--branch` refuses a branch that exists. So a ticket whose work is on a branch (an old
  lane, a run that was stopped, a reopened ticket) can only start over from nothing.

## Design

1. **Send back.** `Command::SendBack { item, reason }`, the controller's (the orchestrator). Taken while the run
   holds no effect in flight: waiting for review, approval or its landing group, or failed. It goes to the
   last build step at or before the current step, never back across a landing (`Land`, `Push`, `AwaitPr`). The
   candidate is dropped and the item queues for its resource like new work, so the sweep starts the build
   again in the same tree: the private tree is named by item, and a named tree is on the item.
   - **Limit:** the workflow sets it, `Definition.send_backs` (0 to 5); left out it is 3. Old pins are
     unchanged, since the field isn't written when left out.
   - **History:** each is recorded on the item (`sent_back`: id, revision, from and to step, run, reason), so the
     ticket's history shows every round. It doesn't touch `iteration` or `back_edges`, which count the
     workflow's own reworks.
2. **The reason reaches the agent.** The item keeps the latest reason (`brief`), from a send-back or a
   rework review, until its build step passes. Every build effect carries it (`Effect.brief`), and `briefed`
   adds "It was sent back to you" with the reason, cut at 1,500 bytes so the launch command stays inside
   tmux's limit. It is part of the effect, so a recovery rebuilds the same command.
3. **Reopen.** `Command::Reopen { item, definition, policy_pin }`, the controller's: a closed ticket (archived,
   or closed this step and not yet archived) comes back as a ticket at rest in the column named (`todo` when
   left out), under its own name, so its history stays in one place. The card's ownership row is permanent and
   already there, and the projection moves the card off Done.
4. **Adopt a branch.** `NamedTree.adopt` (`workflow start --worktree N --branch B --adopt`, and `agent dispatch
   --new N --branch B --adopt`). The branch must exist, here or on a remote, and no other run may hold it. If a
   listed worktree already has it checked out, the run works there (it must be clean at Provision);
   otherwise Provision makes the worktree on it. Main at Start is still the recorded base. The brief tells the
   agent the branch has earlier work and to bring it onto main first. A failed record takes the tree back out
   but never deletes the branch.
5. **CLI.** `workflow send-back WS ITEM --run --pin --reason`, `workflow reopen WS ITEM [--column]`. `agent
   dispatch` on a closed ticket says to reopen it.

## Test

Reducer: send back from review, from a waiting join and from failed; refused while running, across a
landing, past the limit and on a ticket; the reason rides on the next build effect and clears when the build
passes; reopen from the archive and from closed-this-step, refused on a live ticket. Runtime: the brief has
the reason; Start with `--adopt` works in the existing worktree with its commits, and refuses a branch that
doesn't exist or another run holds. CLI: the new verbs map to the wire actions.
