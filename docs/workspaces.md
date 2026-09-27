# Workspaces and worktrees

Far Cooler used to call one worktree plus its branch a *workspace*. That thing
is now called a *worktree*, and *workspace* is kept for something one level
up: a line of work with its own board and its own orchestrator, which owns the
worktrees its agents are using.

Each word gets one job:

> **workspace** — a workstream: its board, its charter, its orchestrator, and
> the worktrees it owns. What a person creates to keep one line of work, and
> one conversation, separate from the rest.
>
> **worktree** — the directory git made and its branch, and everything done to
> one: creating, removing, reviewing its diff, opening it in an editor, running
> an agent in it.

Worktrees are here today. Workspaces arrive with workstreams, which are being
built now: until they land there is no workspace command, no Main, and no
orchestrator a runner records as one, and a repository's single board belongs
to the repository. When they land, every repository gets one workspace, called
Main, and others are split off from it when a thread of work grows large enough
to crowd the rest. Worktrees are disposable: agents make and remove them
freely, and each will belong to at most one workspace.

## Reading the rule off a sentence

Ask what the sentence is *about*. If it is about one directory and branch, or
the terminals, agents, and changes inside it, the word is *worktree*. If it is
about a line of work, its board, or the orchestrator driving it, the word is
*workspace*.

**Worktree**, because the sentence is about one directory and branch:

- `New Worktree`, `Creating worktree…`, `Couldn't create the worktree.`,
  `Created the worktree, but couldn't start Claude Code.` — making one.
- `Remove Worktree…`, `Remove worktree for X?`, `Removing this worktree didn't
  finish.`, `This worktree has uncommitted changes. Enter its name to remove
  it.` — taking one away.
- `Find Worktree or Agent`, `Search worktrees and agents`, `No worktrees on any
  connected runner.`, `worktrees to review` — finding, listing, and counting
  them.
- `A terminal runs one agent, or one shell, inside this worktree.`, `No agent
  is running in this worktree, so there's nowhere to send these yet.` — the
  terminals and agents inside one.
- `Show what this worktree changed, in a pane`, `Nothing uncommitted. The
  worktree is clean.` — its diff, which is how you watch an agent's progress.
- `Open this worktree in your editor`, `Use {path} for the worktree path` — a
  path handed to another program.
- `Already checked out in another worktree` — git's own constraint, in git's
  own words.

**Workspace**, because the sentence is about a line of work. No string says
this yet; these are the sentences that will, once workstreams land:

- The workspace's board, its charter, and its orchestrator — the things that
  are one per workstream rather than one per directory.
- Which worktrees a workspace owns, and which are unclaimed.
- Splitting a thread of work off Main into a workspace of its own, so it gets
  its own board and its own conversation.

A sentence that teaches the relationship needs both words: a workspace owns
worktrees; a worktree belongs to at most one workspace.

## One vocabulary everywhere

The CLI, the wire labels, and the code identifiers follow the same rule as the
copy. There is one user, and every surface updates together, so there is no
older script or older app to keep speaking the old words for. The command is
`farcooler worktree create`, `list`, `adopt`, `branches`, `hide`, `unhide`,
`reorder`, `remove`, and `file-search`; the wire methods are `worktree.*`; and
the call that lists worktrees git has but Far Cooler has not adopted is
`worktree.discover`.

Renames never reach the bytes. Protobuf puts field numbers on the wire, not
names, and those numbers are frozen. A few values are also kept in their old
spelling on purpose, because running terminals and apps already installed
depend on them: the capability values a runner advertises, the tmux tag
`@farcooler_workspace_id`, and the error string `workspaces-exist`.

## Why this is written down

The apps drifted apart on this word once already, and a person who checks the
phone and then the Mac would meet both spellings of the same sentence. A stated
rule holds where a one-time sweep does not: a new string has an answer before
anyone has to argue about it.

All three platforms move together when a string changes sides. The iOS and
Android create-and-fail strings in particular are byte-identical on purpose,
because one person gets whichever surface delivers first.
