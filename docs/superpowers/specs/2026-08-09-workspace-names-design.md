# Design: A workspace is named by its worktree

Date: 2026-08-09
Status: APPROVED (design)
Extends `docs/farcooler-design.md`.

## Problem

A workspace carries two names. `task_name` is prose a person typed, `branch` is
a slug, and the New workspace sheet derives the second from the first
(`Sheets.swift:71`) — so you answer one question twice and then nothing keeps
the answers related. What that produces, from a real database:

| `task_name` | `branch` |
|---|---|
| `review` | `feat/review` |
| `fix-attention-and-reorder` | `worktree-fix-attention-and-reorder` |
| `overnight-sample` | `main` |
| `overnight` | `main` |
| `rate limiting` | `add-rate-limiting-to-the-public-api` |
| `write a haiku about worktrees in HAIKU.md` | `write-a-haiku-about-worktrees-in-haiku-md` |
| `test` | `feat/test` |
| `add tests` | `feat/tests` |
| `refactor api` | `feat/api` |

Three failures are visible in that table. Most titles restate the branch and
earn nothing for having been typed. Worktrees adopted by the reconciler are
named after their directory, so half the sidebar is prose and half is a slug.
And the pairs drift — `add tests` against `feat/tests` — because once both rows
are written nothing ever compares them again.

The fix is not to display the branch instead. A branch is not stable enough to
be a name: one worktree hosts a stack of commits over its life, and the branch
checked out in it changes as the stack is built and rebased. Naming a workspace
after its branch means renaming it whenever the work moves forward, which is
backwards. It also collides — two main checkouts both read `main`.

The directory is the stable thing. A worktree is a directory; that is what it
is. It outlives every branch checked out inside it, it cannot collide with a
sibling because the filesystem forbids it, and it is already the name the
reconciler gives to everything it adopts.

## What we are building

A workspace's name is its worktree directory, read back as prose. Nothing is
stored, and nothing can be renamed.

## The name

```rust
/// What to call a workspace: its worktree directory, read as prose.
///
/// The directory is the only name a workspace has. It is stable across every
/// branch checked out inside it, unique among its siblings because the
/// filesystem says so, and already what the reconciler calls an adopted
/// worktree — so deriving from it makes one rule out of three.
pub fn display_name(worktree_path: &str) -> String {
    Path::new(worktree_path)
        .file_name()
        .map(|n| n.to_string_lossy().replace(['-', '_'], " "))
        .unwrap_or_else(|| worktree_path.to_string())
}
```

`~/…/worktrees/overnight/rate-limiting` reads "rate limiting". The main
checkout lives in the repository's own directory, so `~/Dev/overnight` reads
"overnight" — the rule it already follows, now the only rule, with no
`is_main_checkout` branch to select it.

A detached worktree needs no special case either. The name comes from the
directory whatever the head is, so `branch_of`'s `detached at abc1234` goes back
to being about the branch column alone.

The fallback exists because `file_name` returns `None` for a path with no final
component. No worktree path can be one, but a row showing its full path is
recoverable and a row showing an empty string is not.

## Where worktrees live

New worktrees are created at `worktrees/<repository>/<name>` instead of
`worktrees/<repository>-<name>`. The prefix exists today only because one flat
directory has to keep two repositories' worktrees apart; a subdirectory does
that structurally, and leaves a basename that is already the name we want to
show.

Worktrees created before this change keep their flat paths and keep showing
their prefix — `overnight-fix-attention-and-reorder` reads "overnight fix
attention and reorder". Nothing moves them. This is a design choice rather than
an oversight: moving a worktree behind someone's back is worse than a wordy row
for a few days, and worktrees are meant to be short-lived, so the old shape
empties itself out.

`sanitize` gains two rules it should always have had: collapse runs of `-`, and
trim them from both ends. It maps every character outside `[A-Za-z0-9_-]` to a
dash, so "Rate  Limiting!" currently becomes `Rate--Limiting-`, which was merely
ugly while nothing read it back and becomes wrong now that something does.

It does not lowercase, and must not start. "Rate Limiting" becomes
`Rate-Limiting` and reads back as "Rate Limiting" exactly. Only the branch is
lowercased, in the sheet, as today.

## Creating one

The sheet's Task field becomes the worktree's name, and the sheet shows the
directory path it is about to create beneath the field. That second part is the
point rather than a decoration: nothing encourages careful worktree naming while
the thing being named is invisible.

Typing prose and typing a slug both land on the same path, because
`sanitize("rate limiting")` and `sanitize("rate-limiting")` agree. The branch is
still suggested from the name exactly as it is now.

`validate::task_name` becomes `validate::worktree_name`. It stays at a floor of
one character, drops from 120 to 60, and additionally rejects a name that
sanitizes to nothing. Sixty because this is a path component now, and because a
name generated by an agent — `write a haiku about worktrees in HAIKU.md` — is a
directory nobody wants and a sidebar row nobody can read.

Two workspaces in one repository cannot share a directory, so names are unique
per repository without a uniqueness check: `git worktree add` fails on a path
that exists. That failure should say a worktree of that name is already there,
not repeat what git said about a directory.

The CLI keeps its shape — `farcooler workspace create <repo> <name> --branch …` —
and changes what its help calls the positional.

## Adoption

`adopt_branch` stops taking a name. A worktree created for a branch that already
exists is named after that branch's last segment, which is what anyone would
have typed and what the reconciler would have called it a tick later anyway.

The reconciler stops healing names, because there is nothing to heal. That
deletes `usable_name` and the whole name arm of `reconcile.rs:163-190`,
including its comment explaining why a task workspace's name must *not* follow
git — a carve-out that only existed because a stored name and a derived name
disagreed about who was in charge.

`Store::set_workspace_identity` loses its name parameter and becomes
`set_workspace_branch(id, resource_version, branch, is_main_checkout)`.

## No renaming

There is no rename RPC, no menu item, and no sheet. Two reasons, and the second
is the real one: a name that can be corrected later is a name nobody chooses
carefully now, and a worktree you would want to rename is usually a worktree
that has outlived the change it was opened for.

Typing the name to confirm removing a dirty worktree, and resolving
`workspace remove <name>`, both match the derived name. Neither changes in feel.

## Moving a worktree

`git worktree move` remains the only way to rename, and the reconciler does not
recognize it as a move. It sees one path vanish and another appear: the new path
is adopted as a fresh workspace, and the old row is dropped if it has no
terminals or flagged `worktree_missing` if it has — stranding its terminals and
their agent transcripts under a row whose directory is gone.

We are not fixing that here. We are making it legible. The row says
`worktree gone` and stops there (`SidebarViews.swift:279`), which is enough when
a directory was deleted and not enough when it was moved — the terminals are
fine, they are simply attached to the name the worktree used to have. The badge
gains help text saying a moved worktree is adopted as a separate workspace and
this row keeps the terminals from before the move. Someone who moves a worktree
and finds two rows is told which is which instead of working it out.

## Schema

Migration 0008:

```sql
ALTER TABLE workspaces DROP COLUMN task_name;
```

In the style of 0005, which drops `terminals.loss_dismissed` the same way.

The wire format keeps its `task` field, computed by the daemon from
`worktree_path`. Every shipped client decodes an unchanged shape, so no client
needs releasing alongside the daemon, and `Workspace.task` in Swift and Kotlin
keeps working untouched.

## Testing

- `display_name` over the shapes that occur: a per-repository path, a legacy
  flat path, a main checkout, underscores, and a path with no final component.
- `sanitize` collapses runs, trims edges, and preserves case.
- Creating a workspace puts it under `worktrees/<repository>/`, and creating a
  second one by the same name in the same repository fails with the name-taken
  error rather than a git message.
- `validate::worktree_name` rejects 61 characters and rejects a name that
  sanitizes to nothing.
- Adoption names a worktree from its directory, for a task worktree and for the
  main checkout, through the same path with no `is_main` branch.
- The reconciler leaves the name alone when a branch changes under a worktree —
  the stacked-commits case, which is the reason this design exists.
- Migration 0007 over a database written at 0006 keeps every workspace row and
  every terminal attached to it.

## Not doing

- Detecting `git worktree move` and carrying the row across. Decided against for
  now; see above.
- Moving existing flat worktrees into per-repository subdirectories.
- Lowercasing worktree names. Case is information typed on purpose.
- A nickname, alias, or label field by another name. The point is one name.
