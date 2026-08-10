# Design: the worktree workbench

A worktree is a high-level objective. Its objects are agents and changes; its
actions are prompt, review and change. The app's shell should say that.

Today it does not. The main area is a terminal viewer — selecting a worktree
shows its tmux tiling, selecting a terminal shows that terminal — and everything
that is not a terminal has been bolted to the side. Review arrived as a trailing
inspector, which put code review in a 300pt column, and diff status arrived in a
sidebar row that has no room for it.

## What the app is

Far Cooler is where you take something from an idea to a merged PR.

That is a deliberate widening. The original design called it "a terminal-first
command center", and terminal-first described how it started rather than what it
is for: terminals are how agents happen to expose themselves, not the thing
anyone cares about. The span the product owns is prompting, running, reading,
reviewing, stacking and landing.

This document specifies the **shell** those phases live in — the information
architecture, not the phases. Each phase's own surface (PR stacking, landing,
ticket import) is its own spec.

## What stays the unit

The worktree. Considered and rejected: an `Objective` object above it that could
exist before its worktree, hold several of them, and outlive them. It would have
carried unstarted work, but unstarted work has a home already, and inventing a
resource so the app can hold a to-do list is the wrong trade. A ticket becomes a
**creation path** — "new worktree from this ticket" — not a new resource.

## Architecture

### Tiles

A worktree's main area is a client-owned tree of tiles.

```
┌─ workspace ────────────────────────────────────────────┐
│ ┌─ TmuxCanvas ───────────┐ ┌─ Diff(Branch) ──────────┐ │
│ │ tmux owns everything   │ │ jump bar                │ │
│ │ inside this box:       │ │ ─────────────────────── │ │
│ │  ┌────────┬─────────┐  │ │ one long scroll of      │ │
│ │  │ agent  │ shell   │  │ │ every changed file      │ │
│ │  └────────┴─────────┘  │ │                         │ │
│ └────────────────────────┘ └─────────────────────────┘ │
└────────────────────────────────────────────────────────┘
```

Tile kinds: `TmuxCanvas`, `Diff`, `Pr`.

**`TmuxCanvas` is one tile, and tmux keeps its own tiling inside it.** This is
the constraint that makes the whole design affordable. Splitting terminals still
goes through `layout.split`; `PaneGroup` and `PaneRect` remain a projection of
tmux's window tree, keyed by `terminal_id`, exactly as they are. We are building
an **outer** layout whose leaves are either tmux's canvas or a view of our own.

The alternative — every terminal individually placeable alongside diffs — was
rejected. It requires the client to own arrangement for live processes, which is
a second pane tree beside tmux's. Migration 0003 deleted exactly that, and the
reasoning has not changed: tmux has had split trees, named layouts, dividers and
zoom for twenty years, it is already the authority for what is running, and a
second copy means a third in every client that draws it.

**The layout is client-local**, persisted per workspace per device.

Not daemon-owned, and this is the call in this document I hold most loosely. For:
tile arrangement is view state rather than intent about the work; a phone shows
one tile at a time regardless, so syncing arrangement buys almost nothing; and
daemon ownership means new messages, storage, events and a cross-device conflict
rule for something whose default most people never change. Against: "the daemon
owns authoritative state" is a real principle here. Revisit if it bites.

**Consequence: no protocol change.** Every `layout.*` method is untouched. Diff
and PR tiles read from RPCs that already exist.

### Default layout

A worktree opens with `TmuxCanvas` on the left and `Diff(Branch)` on the right.

The diff beside the agent that is changing it is the most valuable adjacency in
the whole product, and it should not be something you have to arrange.

## Tiles in detail

### Diff tile

One concept that reshapes at two widths, not two modes to learn.

- **Scope selector** in the header: `Branch` (base…HEAD), `Commit` (pick one),
  `Local` (uncommitted). These map onto the existing `DiffSelector` —
  `Range`, `Commit(sha)`, `Staged`/`Unstaged` — so the daemon needs nothing new.
- **The body is one long scroll of every changed file**, in order. Not
  file-at-a-time: skimming a branch is the first thing you do, and a viewer that
  makes you open files one by one turns skimming into forty clicks.
- **A quick jump bar** pinned at the top: every changed file with its `+N −M`,
  click to scroll there.
- **Wide: the jump bar is promoted into a persistent left column.** Files and
  counts on the left, diff on the right, GitHub-shaped. Same information with
  more room — deliberately the same control at both widths, so the narrow layout
  teaches the wide one.
- No syntax highlighting, still. What earns the pixels is which lines changed.

```
narrow                          wide
┌──────────────────────────┐    ┌──────────┬───────────────────────┐
│ a.rs +12 −3 · b.rs +4 −0 │    │ a.rs     │ @@ -1,3 +1,4 @@       │
├──────────────────────────┤    │  +12 −3  │  fn main() {          │
│ @@ -1,3 +1,4 @@          │    │ b.rs     │ -    old()            │
│  fn main() {             │    │  +4  −0  │ +    new()            │
│ -    old()               │    │          │  }                    │
└──────────────────────────┘    └──────────┴───────────────────────┘
```

### PR tile

Graphite-shaped, because the thing a PR view has to do here is show a stack.

- Header: title, state, checks, review decision.
- **The stack, drawn as a stack**, current slice marked. A worktree routinely
  carries several PRs; this is the tile's reason to exist rather than a link to
  a browser.
- Description body.
- Commits list; selecting one drives a diff inside the tile using the Diff
  tile's renderer.

**Degradation is honest and visible.** Description, checks and review state come
from `gh`. A machine without it shows the local stack and commits and says the
rest is unavailable — it does not show an empty PR and let you conclude there
isn't one.

### Deleted: the review buffer

Captured comments, anchors, dispatch and the outbox are removed entirely.

The buffer existed because accumulating comments was unsupported anywhere: you
noticed something, held it in your head, and typed it into an agent later. With
an agent tile beside the diff, you type it as you notice it. The insight that
produced the buffer survives — a review comment aimed at an agent is a cheap
instruction, not a message — but the machinery was a workaround for a layout
problem, and the layout is now fixed.

Removed: `review_entries`, `review_dispatches`, `review_attachments`,
`review_viewed`; the anchor system and its resolution ladder; capture manifests
and range-based re-read; the dispatch outbox and numbered answer correlation;
`review.capture/update/delete/list/dispatch/mark_viewed/attachment_*`;
`farcooler review note/list/drop/send`; the sidebar's needs-you badge; and the
inbox's open/answered/unknown counts.

Kept: `review_reviewed` and `worktree_digest`, which are not about comments —
they are how the fleet knows a worktree moved since you last read it.

## Sidebar

Stays a navigator, and gets cheaper rather than richer.

Projects → worktrees, one line each: task name, branch, `+N −M`, a dot when an
agent wants you, and a marker when it moved since you last read it. No diff
content. There may be hundreds of these rows, and the row must stay the lightest
thing in the app.

A **Fleet** item sits at the top. Selecting nothing shows the fleet view.

## Fleet view

The main area when no worktree is selected. It answers "what have I started and
not finished", which is a to-do list whose items are worktrees.

Grouped by what the work is doing:

| Group | Contains | The row's job |
|---|---|---|
| **Working** | agents running | what the agent is doing, how long, `+N −M` so far |
| **Waiting on you** | an agent asked something | what it asked |
| **Parked** | no agents, has changes, not landed | re-orientation |
| **Landing** | open PRs | checks and review state |

**Parked is the group that earns this screen.** Coming back to work you left a
day ago costs re-orientation, and a row that only says its name costs you a
worktree visit to find out anything. So it carries: task name, last commit
subject and when, `+N −M`, **how far behind base it now is**, and PR state if
there is one. The last two together are what tell you whether resuming is cheap
or whether it has rotted.

Everything on these rows exists in the daemon already: change sets, `shortstat`,
the stack, PR status, and agent activity from `watch.rs`.

## Phones

iOS and Android do not tile. One tile at a time from a segmented control —
Agents · Changes · PRs. The tile *kinds* are shared; only the arrangement
differs, and the narrow Diff layout is the one the tiles were designed around
first for exactly this reason.

## Naming

`review` becomes `changes` across the CLI and the daemon methods. With the
buffer gone the surface is not review any more, and renaming an unreleased
surface is cheap now and annoying later.

## Testing

The Rust suite keeps everything except the deleted buffer tests. New seams worth
testing:

- the scope selector → `DiffSelector` mapping, including that `Local` covers
  staged and unstaged;
- the fleet grouping rule — which group a worktree lands in, and specifically
  that a worktree with a running agent and an open PR appears once, in Working;
- layout persistence round-tripping, including a layout naming a tile kind a
  newer build introduced.

The responsive breakpoint and tile chrome get a manual pass. SwiftUI layout is
not worth a harness here, and the existing suite has no precedent for one.

## Build order

Five pieces, each shippable on its own. The order matters because the tile
system is the substrate everything else sits in, and the deletion makes the
substrate smaller before it is built.

1. **Delete the buffer, and rename.** Subtractive, and it shrinks what the tile
   system has to host. Doing it first means the rename touches less code.
2. **The tile system, plus the Diff tile and the default layout.** The core.
   After this the original complaint is fixed: review has the main area, beside
   the agent changing it.
3. **The fleet view**, which needs no new backend and answers the to-do-list
   question.
4. **The PR tile.** Last of the desktop work because it is the only piece that
   depends on `gh`, and the only one that is half-empty without it.
5. **The phone segments**, once the tile kinds have settled on the Mac.

Stopping after 2 leaves a coherent product. Stopping after 3 leaves a good one.

## Not in scope

| Deferred | Why |
|---|---|
| Linear/Jira import | A creation path, and its own spec. The model change it would have forced was rejected above. |
| An `Objective` above the worktree | Rejected: unstarted work lives in the tracker. |
| Writing to GitHub | The product reads PR state; creating and merging stays manual. |
| Individually placeable terminals | Rebuilds the pane tree migration 0003 deleted. |
| Daemon-owned tile layout | Client-local until it bites. |
| Comment capture on mobile | Deleted with the buffer. Typing at an agent is the path on every client. |

## Open questions

1. **Where does the fleet view's "Waiting on you" get its evidence?** The
   original design was explicit that events may only carry hard facts the daemon
   observed, and that "needs input" cannot be established from generic PTY
   output. For ACP agents the structured stream gives it; for a bare shell it
   does not. The group may have to be ACP-only, and say so.
2. **What closes a worktree?** "Landing" implies a terminal state, and nothing
   currently moves a worktree out of the fleet view except deleting it.
3. **Does the Diff tile's `Commit` scope need a picker, or does the PR tile own
   commit selection?** Two ways to reach the same view.
