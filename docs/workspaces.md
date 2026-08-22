# Workspaces, and when to say *worktree*

**A workspace is one git worktree plus one branch for one task,** along with its
terminals, its agents and its diff. It is the thing you open, switch to, count
in a list, and find in the command palette.

Both words are real and both stay. The worktree is the directory git made; the
workspace is everything Far Cooler hangs off it. Collapsing them costs
precision in both directions: "Remove Workspace" would promise to take away the
terminals and the branch, which that command does not do, while "Search
worktrees and agents" offers to search directories when what you are actually
searching is tasks.

So each word gets a job:

> **workspace** — anything a person navigates, lists, counts, searches for, or
> switches between. The container: its terminals, its agents, its diff.
>
> **worktree** — the directory on disk and the git object itself, especially
> where it is being created or destroyed, and where git's own constraints are
> being reported.

## Reading the rule off a sentence

Ask what the sentence is *about*, not what mechanism is underneath. Almost every
string in the product sits on a worktree somehow; that is not what decides it.

**Workspace**, because the reader is moving around the product:

- `Find Workspace or Agent`, `Search workspaces and agents`, `Go to a terminal,
  a workspace, or start something` — navigation and search.
- `Workspaces` as a screen title, `No workspaces on any connected runner.`,
  `Hide hidden workspaces`, `workspaces to review` — lists and counts.
- `A terminal runs one agent, or one shell, inside this workspace.` — the
  container holding terminals.
- `Show what this workspace changed, in a pane`, `Couldn't read this workspace`,
  `Nothing uncommitted. The workspace is clean.` — the workspace's diff. The
  directory is an implementation detail of reading it.
- `No agent is running in this workspace, so there's nowhere to send these yet.`
  — again the container, this time holding agents.

**Worktree**, because the sentence is about the git object:

- `Creating worktree…`, `Couldn't create the worktree.`, `Created the
  worktree, but couldn't start Claude Code.` — the moment git makes the
  directory, which is exactly where the word earns its keep. (`New Workspace`
  still names the *command*: you are making a workspace, and the progress row
  narrates the part git is doing.)
- `Remove Worktree…`, `Remove worktree for X?`, `Removing this worktree didn't
  finish.`, `worktree gone` — destructive, and naming the directory is the
  point.
- `Already checked out in another worktree` — git's own constraint, reported
  in git's own words.
- `Open this worktree in your editor`, `Use {path} for the worktree path` — a
  path handed to another program.
- `A workspace contains one Git worktree and branch.` — the sentence that
  teaches the relationship needs both words, and is the reason both exist.

One exception inside the remove flow: `This workspace has uncommitted changes.
Enter its name to remove it.` says workspace, because the name it asks you to
type is the workspace's, and because the same sentence appears as a sidebar
tooltip where the reader is only browsing. Two spellings of one sentence on two
surfaces is the failure this rule exists to prevent, so the sentence picks one
and keeps it. Everything else in that sheet — the title, the button, `deletes
the working directory`, `Removing this worktree didn't finish.` — still names
the directory.

## What does not rename

**Command names and flags.** `farcooler workspace remove-worktree` keeps its
name, and so does every flag, for the reason [`runners.md`](runners.md) gives
about `--host`: they live in shell history and in scripts, and a vocabulary
change is not a reason to break either. Reader-facing `println!` copy follows
the rule; the grammar does not.

**The wire protocol.** `workspace.remove_worktree`, `worktree.file_search` and
the `worktree_missing` state are on the wire. Renaming those needs a version and
a compatibility window, not a vocabulary decision — the same answer the design
doc's vocabulary note gave for the proto's `Host` message.

**Code identifiers, and most internal comments.** `removeWorktree`,
`WorktreeName`, `HiddenWorktrees`, `worktreePath` and their kin stay as they
are. This rule governs the words a person reads; several of these identifiers
name the CLI command or the wire method they call, and renaming the rest would
be a large diff with nothing to show for it on screen. Comments follow the rule
where they explain a string, name a screen, or would otherwise contradict the
copy beside them — not as a blanket sweep.

## Why this is written down

The three apps drifted apart on this word once already, and a person who checks
the phone and then the Mac meets both spellings of the same sentence. The fix
that holds is a stated rule rather than a one-time sweep, so a new string has an
answer before anyone has to argue about it.

All three platforms move together when a string changes sides. The iOS and
Android create-and-fail strings in particular are byte-identical on purpose —
one person gets whichever surface delivers first.
