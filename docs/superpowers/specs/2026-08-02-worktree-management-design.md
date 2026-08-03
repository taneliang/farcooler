# Design: Far Cooler manages worktrees

Date: 2026-08-02
Status: APPROVED (design); implementation not started
Extends `docs/farcooler-design.md`.

Followed by a separate project — see "The next project" at the end — which puts
every machine's projects in the same scroll area. This design deliberately does
not do that.

## Problem

Far Cooler only ever knows about work it started itself.

Registering a repository creates zero workspaces. Every worktree already on
disk — and people arrive with several, because using multiple worktrees is
precisely the habit this product is for — has to be brought in by hand through
an "Import existing worktrees" sheet the user has to know exists. The
repository's own checkout is a third thing again, adopted through a separate
gesture (`workspace.main`) that exists because it was otherwise the one
directory the app could not open a terminal in.

The result is a tool that, in its first hour, asks you to re-create by hand
what is already on your disk. Then it drifts: a `git worktree add` typed in a
terminal never appears, and a `git worktree remove` leaves a row pointing at
nothing.

The mental model should be the one the user already has. They think in
worktrees. Far Cooler should show them their worktrees.

## What we are building

**Git is the source of truth for which worktrees exist.** The `workspaces`
table becomes a cache keyed by canonical worktree path, holding only what Far
Cooler knows and git does not: terminals, layouts, agent sessions, and whether
the user has hidden the row.

Every worktree `git worktree list` reports is one sidebar row, main checkout
included. There is no import step, no adopt gesture, and no way for the sidebar
and git to disagree for longer than one reconcile tick.

## What already exists

Most of the git plumbing is done and does not change:

- `git::list_worktrees` — `--porcelain` parsing, with `is_main`, `locked`, and
  `prunable` already surfaced (`crates/daemon/src/git.rs`).
- `Service::import_worktree` — registers a worktree without touching git.
  Becomes an internal function called by the reconciler.
- `Service::remove_worktree` — real `git worktree remove --force`, refuses the
  main checkout, refuses while a managed terminal runs, never deletes the
  branch.
- `Service::archive_workspace` / `restore_workspace` — hides without touching
  git. Becomes hide/unhide.

What is missing is the model, the reconciliation, and the sidebar.

## Reconciliation

New module: `crates/daemon/src/reconcile.rs`.

One pass over one repository:

1. `git::list_worktrees` for the repository.
2. Skip `prunable` records. The directory is already gone and git will drop the
   record itself; adopting one would create a row for a worktree that does not
   exist.
3. Any git worktree with no row at that canonical path → create a workspace,
   named after its directory. That is what whoever made the worktree already
   chose to call this work.
4. Any row whose path git no longer lists, or whose directory is gone:
   - **Zero terminal records** → delete the row. It disappears.
   - Otherwise → set `worktree_missing` and keep the row, rendered as "worktree
     gone" with a Dismiss action that deletes it.

"Empty" means zero rows in `terminals` for that workspace, which is the whole
test. Agent sessions hang off terminals (`terminals.agent_session_id`), so a
workspace with no terminals has no transcript either, and there is no second
condition to get out of step with the first.

Point 4 is the only place this design destroys a record, and it is bounded to
records that carry nothing. A worktree with an agent transcript in it is never
removed behind the user's back on the strength of a `git worktree list` that
might have raced a `mv`.

### Cadence

Folded into the existing `Watcher::run` loop in `crates/daemon/src/watch.rs`,
which already ticks every second and already exists to be the one place the
daemon samples the world for all clients.

Spawning `git worktree list` per repository per second is waste. Both
`worktree add` and `worktree remove` mutate `$GIT_COMMON_DIR/worktrees/`, so
the tick stats that directory's mtime and only scans repositories whose mtime
moved, with a forced full pass every 30 seconds as a backstop against anything
that changes a worktree without touching that directory.

`repository.register` reconciles immediately and synchronously, so adding a
project fills the sidebar before the sheet closes.

### The race, which is real today

`create_workspace` runs `git worktree add` and *then* writes the row. A
reconcile landing in that window sees a worktree with no row and imports it
under the directory name; the original call then writes a second row for the
same path.

`git.rs`'s module header states that workspace creation "is serialized per
repository". No such lock exists — the comment describes an intent that was
never implemented, and nothing depended on it until now.

So this design adds:

A per-repository `tokio::Mutex` on `Service`, held by `create_workspace`,
`adopt_branch`, `remove_worktree`, and each reconcile pass — plus the unique
index in the schema above, which turns a future mistake into an error rather
than a duplicate row.

### The main checkout

Reconciliation creates it like any other worktree. It gets an explicit
`is_main_checkout` column, which drives a pin glyph and the absence of a Remove
item in its menu.

The flag replaces a string comparison. `Workspace.isMainCheckout` in
`apps/macos/Sources/FarCooler/Model.swift:59` is currently `task == "main"`,
which a linked worktree whose directory is named `main` would defeat. Under
auto-import that collision stops being hypothetical, and the thing it guards is
whether the UI offers to delete the directory the user works in.

`Service::main_workspace` and the `workspace.main` RPC are deleted. Their whole
job was to make this row exist on demand; it now always exists.

## Schema

One forward migration on `workspaces`:

| Change | Why |
|---|---|
| `archived` → `hidden` | Rename, see below. Existing archived rows land hidden. |
| `+ is_main_checkout INTEGER NOT NULL DEFAULT 0` | Replaces a name comparison with a fact. |
| `+ worktree_missing INTEGER NOT NULL DEFAULT 0` | Set by the reconciler; fed into `derive_workspace`. |
| `+ UNIQUE(repository_id, worktree_path)` | Backstop against a duplicate row for one path. |

`worktree_missing` is stored rather than derived because the reconciler is the
only thing that knows: `derive::derive_workspace` runs on every read and has no
business shelling out to git. It feeds a new
`WORKSPACE_STATE_WORKTREE_MISSING = 6` in the proto enum.

## Hide replaces archive

One concept, not two. Archiving today means "hide without touching git", which
is what hiding means, so the rename is the whole change:

- Store: `archived` → `hidden`, per the migration above.
- Proto: `WORKSPACE_STATE_ARCHIVED` → `WORKSPACE_STATE_HIDDEN`, keeping tag 5.
- RPC: `workspace.archive` / `workspace.restore` → `workspace.hide` /
  `workspace.unhide`.
- CLI: `workspace archive` / `restore` → `workspace hide` / `unhide`.

Hidden is a property of the row, so it survives reconciliation. A hidden
worktree stays hidden; it is not re-adopted as visible on the next tick.

**Hiding no longer refuses while a terminal is running.** Archive refuses today,
which was right when archiving meant "done with this". Hiding is a view
preference, and a view preference that fails with an error reads as a bug. The
risk it was guarding — losing track of a running agent — is handled where it
belongs: the `Hidden (n)` header carries an attention dot when anything inside
it wants the user.

### In the sidebar

Hidden rows leave the main list. A collapsed `Hidden (n)` disclosure sits at
the bottom of its project's section, collapse state remembered per project.
Expanding shows the same rows, dimmed, each with Unhide.

## Remove means remove

The semantics are already correct and do not change: `git worktree remove
--force`, the branch always survives, the main checkout is refused. What changes
is that removal stops being an emergency action.

- The context menu item is `Remove Worktree…`, present on every row except the
  main checkout, where it is absent rather than refused. A daemon-side refusal
  is a safety net, not a design.
- The confirm sheet states plainly that the directory is deleted and the branch
  is not.
- **Typed-name confirmation only when the worktree is dirty.** A clean worktree
  with nothing uncommitted gets an ordinary confirm button. Requiring the name
  to be typed for routine cleanup trains people to type it without reading it,
  which spends the one gesture that should mean something.

## What is deleted

- `apps/macos/Sources/FarCooler/ImportWorktrees.swift` and its sheet.
- `WorktreeImport` proto message, the `workspace.import` RPC, and the
  `workspace import` CLI subcommand.
- `workspace.main` RPC, its CLI subcommand, and `Service::main_workspace`.
- The "Import existing worktrees…" item in the sidebar `+` menu.

`worktree.list` stays as a host-admin diagnostic. `Service::import_worktree`
stays as the reconciler's internal adopt function.

## Data flow

```
Watcher tick (1s)
  └─ for each repository whose $GIT_COMMON_DIR/worktrees mtime moved
       └─ reconcile_repository(repo_id)          [holds the repo's mutex]
            ├─ git worktree list --porcelain
            ├─ adopt unknown paths      → store.create_workspace
            └─ resolve vanished rows    → delete, or flag worktree_missing
                 └─ broadcast Event so every client updates without polling
```

Clients render. No client computes which worktrees exist, for the same reason
no client computes terminal state today: two authorities on one question
disagree, and the user is the one who finds out.

## Testing

Reconciler, against real temporary repositories rather than fixtures — the
shape of git's output is the thing under test:

- A worktree added outside Far Cooler appears after one pass.
- A removed worktree with no terminals disappears.
- A removed worktree with terminals survives, flagged missing.
- The main checkout appears exactly once, flagged, and never twice across
  repeated passes.
- A hidden worktree stays hidden across a pass.
- `create_workspace` racing a reconcile produces exactly one row.
- A prunable record is not adopted.

Store:

- Migration test: a database written before this change opens, and its
  `archived` rows land as `hidden`.
- The `UNIQUE(repository_id, worktree_path)` index rejects a duplicate.
- `is_main_checkout` defaults to 0 for pre-existing rows and is corrected by
  the first reconcile pass.

## The next project: every machine in one scroll area

Agreed and specced separately, because it is a client rearchitecture rather
than a daemon change.

The "This Mac" dropdown is not a filter. `Preferences.remoteHost` is what the
app puts in `--host` on every CLI invocation
(`apps/macos/Sources/FarCooler/DaemonClient.swift:32`), so the app talks to
exactly one daemon at a time. Dropping the picker means holding N concurrent
connections, merging their fleets, and routing every mutation back to the
machine its row came from.

Decisions already taken, to carry into that spec:

- Projects are equal regardless of host. The host is shown alongside each
  project, not selected globally.
- An unreachable machine keeps its projects visible and dimmed, badged
  unreachable, retrying in the background. They do not vanish as a laptop
  sleeps.

Two facts that make it cheaper than it looks: `crates/cli/src/main.rs:947`
already stamps `"host"` into every workspace from the `--host` flag it was
invoked with, and `ContentView.swift:281-289` already groups by `project ·
host` when more than one host is present. The merge works the moment more than
one fleet arrives.

This design leaves that grouping code in place rather than simplifying it away.
