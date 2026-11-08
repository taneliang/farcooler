# ov-967: runs use visible, named worktrees

From the owner's answer to ov-964 item 2: the orchestrator names the worktree and branch, the app lists
it, and it starts from main as it is at Start. Stacked on ov-966, which made Start take the model.

## Where it stands

- Provision makes `<runtime root>/tree-<digest>` on branch `wf-<digest>` (`coding_execute.rs:219`,
  `adapters.rs:129`). The runtime root is the daemon's private `workflow/` directory, so the app never shows
  it, and no `worktrees` row is written or claimed.
- **Correction to ov-964's "frozen at publish":** Provision's base is already read from the landing ref when
  the step is admitted (`adapters.rs:96`), not the published `base`. What's missing is that it isn't
  recorded on the run, and it is read at Provision rather than at Start.
- The workflow runtime has the `Store` but not the `Service`, which owns the worktrees directory, the
  per-repository lock the reconciler takes, and the install id that marks a worktree as this install's.

## Design

1. **Start.** `Action::Start` gets `worktree: Option<{ name, branch }>`. The daemon's Start handler checks the
   names as `worktree.create` does, refuses a branch that already exists or a worktree directory that
   does, reads the landing ref (main), and sends `Command::Start` a `tree: { name, branch, path, base }`.
   The reducer records it on the item (`Item.tree`), and every effect of the item carries it
   (`Effect.tree`). Skipped when absent, so old runs, pins and receipts are unchanged.
2. **Provision.** With a `tree`, it makes the worktree at `path`, on `branch`, from `base`, refusing an
   existing branch (`fork_only`), under the repository's lock. It marks it as this install's, writes the
   `worktrees` row and claims it for the workspace explicitly. The base it pins is the recorded one, so the
   run starts from main as it was at Start.
3. **Every later step** finds the tree at `effect.tree.path`, and the branch check reads `effect.tree.branch`.
4. **Service.** `start_workflows` hands the runtime what it needs: the worktrees directory, the install id
   and the repository locks it shares with the reconciler and `worktree.create`.
5. **CLI.** `workflow start --worktree NAME --branch B`. `agent dispatch --new NAME --branch B` on one tracked
   ticket sends them; with more than one ticket it's refused, as each run needs its own. `--worktree` (an
   existing tree), `--base`, `--title` and `--again` stay refused: the base is main at Start.
6. **Without a name**, a run keeps the private tree. The orchestrator names one when it wants it visible.

## Test

Acceptance, over the socket with a real Service: publish, move main on, Start a tracked ticket with a
name and branch. The app's worktree list has it, claimed for the workspace, on that branch, and the run
records main's commit at Start as its base. A Start naming an existing branch is refused and makes nothing.
CLI: `--new`/`--branch` map to the Start; refusals for several tickets.
