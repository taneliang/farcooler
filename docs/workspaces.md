# Workspaces and worktrees

A **workspace** is one line of work in a repository. It has its own board of
tasks, its own charter (the rules for how that work gets done), and its own
orchestrator: the lead agent you talk to about it. You keep one line of work,
and one conversation, separate from the rest by giving it a workspace.

A **worktree** is a directory git made, with its branch. Agents do their work in
worktrees, and a workspace owns the worktrees its agents are using.

Every repository starts with one workspace, called Main. When a thread of work
grows large enough to crowd out the rest, you split it off into a workspace of
its own. A runner too old to know about workspaces still shows its repository's
single board, with no orchestrator and no rail.

## The words

Six nouns, each with one meaning, on the Mac, the phones and the command line.

- **Workspace**: a line of work, with its board, its charter, its orchestrator,
  and the worktrees it owns.
- **Orchestrator**: a workspace's lead agent. It reads the charter, puts work on
  the board, dispatches agents, and asks you when it needs a decision. It
  manages the work; it doesn't do it. A workspace has at most one.
- **Board**: a workspace's tasks, by status: Backlog, To Do, In Progress, Needs
  Decision, In Review, Done and Canceled.
- **Task**: one card on a board, with a key such as `bil-9`.
- **Agent**: a coding agent (Claude, Codex or Cursor) running in a terminal. A
  terminal that runs a shell instead is just a terminal.
- **Worktree**: a directory and its branch, and everything done to one:
  creating it, removing it, reviewing its changes, opening it in an editor. The
  repository's own checkout is its **main checkout**.

A workspace owns worktrees; a worktree belongs to at most one workspace. A
worktree no workspace owns is **Unclaimed**. Worktrees are disposable: the
orchestrator makes one when it dispatches a task and removes it when the work
lands.

## What needs you

**Needs You** is one list of everything waiting on you, from every workspace on
every runner. It sits at the top of the Mac's sidebar and is the first screen on
the phones. Each item names its workspace, and there are four kinds, most urgent
first:

- **An agent asking permission**, such as to run a command. Answer it in place
  with **Allow** or **Deny**.
- **A blocked agent** that needs you at its terminal. **Open** takes you there.
- **A decision** the orchestrator has asked you for. Answer it in place with one
  of its options, or with **Answer…** when it has none. Your answer goes on the
  task, and the orchestrator picks it up from there.
- **A task ready for review.** **Review** opens its changes. Needs You never
  approves work for you: the charter says who lands it.

An item leaves the list once it's answered. If you answer a permission ask that
someone else already answered, the item keeps the line "Someone already
answered this." under its buttons until the list catches up.

The count beside Needs You is the number of items, and it's the same number the
lock screen, the widget and the watch show. Each workspace row in the sidebar
carries its own share of that count.

Some things aren't items, on purpose. An agent that finished its turn keeps its
checkmark and its notification, and a worktree with unread changes keeps its dot,
but neither waits in Needs You. An orchestrator finishes every turn, so its
finished turn is only an unread dot on its workspace.

To walk the list without leaving the keyboard, press ⌃⌘N (Terminal ▸ Next
Needing Attention) on the Mac. From a terminal, `farcooler needs-you` prints the
same list.

## A workspace, on the Mac

The sidebar lists each repository's workspaces, with Needs You above them. A
workspace row shows its name, its orchestrator's status (`◌` when it has none),
and how many items need you. Its menu has **Show Board**, **Start
Orchestrator** (or **Replace Orchestrator**) and **Show Charter**.

Select a workspace to see its board. The orchestrator sits on a thin rail at
the left edge, with its status and its dot; click the rail (or press ⌥⌘1) to pop
its conversation open over the board, and click the rail again, click anywhere
outside it, or press ⌥⌘1 again to put it away. Left to right, it's a
chain: the orchestrator, then the board, then whatever you've opened from it.

**Talking to the orchestrator.** The conversation is the orchestrator's own
terminal, or a chat view when it's in chat mode. The `⋯` menu in the
conversation's header switches with **Show as Chat** or **Show as Terminal**,
and also has **Replace Orchestrator**, **Show Charter**, **Restart** and **Stop
Being Orchestrator**. Tell it what you want done in plain words. It puts the work
on the board, dispatches agents, and reports back from the board, not from
memory. It writes down what it decides, so its work survives a restart. The
first time you talk to a new orchestrator, it interviews you for the charter:
how work gets from idea to landed, what done means, who reviews, what it may
decide alone. You can edit the charter any time with **Show Charter**; the
orchestrator rereads it whenever it picks the work back up.

**The board** is a list sectioned by status, with Needs Decision first. When a
workspace has tasks waiting on a decision, the header says "3 tasks are waiting
on you" ("1 task is waiting on you" for one), and "3 waiting" where the room is
short. It says nothing at zero. Every status is always shown, even an empty one:
an empty status is a header with a 0. Done and Canceled start collapsed. The
**+** button in the board's header (**New Task…**) files a task by hand. It's
there when the runner lets this Mac write to the board. Beside it, the arrow
re-reads the board.

## Following a task to its changes

Select a task, from the board, the sidebar, Needs You or ⌘P, to open it. The
board narrows to a list on the left, with the task selected, and the task opens
beside it, under a breadcrumb: its key, title and status first, then everything on its card, never collapsed: what it's for, what
counts as done, its notes, and, when it's waiting on a decision, the question
with its answers. The status menu in the header moves the task.

Beneath the task, past a divider you can drag, is its work: a line naming its
worktree and how many terminals it has, the agent working on it, and that
worktree's **Changes**. When the task is In Review, the changes take the larger
share. A task with nothing started is just that line.

- **Open Worktree** shows the worktree itself, full size, with all its terminals
  and layouts. The breadcrumb leads back to the task.
- A task with no agent still reaches its worktree and changes, so a finished
  task in review is one click from its diff.
- To glance at another task, click it in the list, or press ↑ or ↓ while the
  list has the keyboard: it swaps in place, and the list keeps its scroll.
- To close the task, click it again in the list, click the **×** at the end of
  the breadcrumb, or press Esc (when no terminal or field has the keyboard).
  The board widens back. Return in the list, with a task open, moves into it.
- Drag the line between the list and the task to make the list wider or
  narrower; it remembers the width. In a window too narrow for both, the task
  covers the board instead, and closing it brings the board back as it was.
- The orchestrator's rail stays at the left, and pops open over the task the
  same way. Esc stays with the terminal you're typing in.
- Back (⌃⌘←, in the Workspace menu) goes up one level, from a worktree to the
  task it was opened from, and from a task back to the board. The breadcrumb
  goes straight to any level. Focus (⌃⌘↩) gives the agent and changes the whole
  view, without the rail or the board; press it again to bring them back.
- ⌥⌘1, ⌥⌘2 and ⌥⌘3 give the keyboard to the orchestrator, the board's list and
  the task.

Opening, closing and switching all move on one spring, and you can click again
while something is still moving.

A worktree with no workspace of its own opens the same way, beside its
repository's board, with that board's orchestrator on the rail.

## Worktrees

A worktree isn't a place of its own anymore; you reach it from what it's for.

- **Under a task**, with **Open Worktree**.
- **Under a workspace's sidebar row.** Its chevron lists every worktree the
  workspace owns, each with its task key and its line counts, including ones
  with no task, such as a shell in a branch of your own. Clicking the row
  itself still opens the workspace. Expanding a worktree lists its terminals,
  one step further in. **New Worktree…** is on the workspace row's shortcut
  menu.

To move a worktree to another workspace, drag it onto that workspace's row, or
Control-click it and choose **Move to Workspace**. From a terminal, use
`farcooler worktree assign`. Worktrees no workspace owns are under Unclaimed,
and hidden ones under Hidden.

Nothing about a worktree itself changed: its terminals, layouts and tmux
commands work as they always have, in whichever view has focus. On the Mac,
**Show Changes** is on a worktree row's shortcut menu and its **…** menu, and
**Changes** is in the toolbar while a worktree is open, whether or not it has a
terminal: with none, its changes pane opens on its own.

## Starting an orchestrator

A workspace with no orchestrator says so, with **Start Orchestrator**. Choose
Claude, Codex or Cursor, and it starts working on its own: it reads the charter
(or interviews you for one, if there isn't one yet) and then the board. You
don't need to type anything to get it going. You can start one from the
workspace's row in the sidebar, from its conversation (pop it open from the rail),
or on either phone.

If an orchestrator's terminal is lost, its conversation (pop it open from the
rail) says "The orchestrator stopped". **Restart** picks the conversation back up where it left
off, and **Replace…** starts a new one. Replacing asks you to confirm first,
because the orchestrator running now closes. From a terminal:

```
farcooler workspace start-orchestrator Billing --harness claude
farcooler workspace start-orchestrator Billing --harness codex --replace
```

## Adopting an orchestrator you already started

If you already have an agent running in a repository's main checkout, you don't
have to start another. Make it the orchestrator:

- **On the Mac**, Control-click its terminal in the sidebar and choose **Use as
  Orchestrator**, or choose **Use a Running Terminal…** beside Start
  Orchestrator in an empty conversation column. It's offered only for a running
  terminal that belongs to a workspace and sits in the main checkout, never for
  a task's agent.
- **From a terminal**, run `farcooler terminal set-role <terminal>
  orchestrator`. The same command takes `agent` and `shell`.

A workspace has at most one orchestrator. If it already has one, the Mac asks
whether to replace it: the old orchestrator keeps running as an ordinary
terminal, and yours takes over the board. The command refuses instead, and you
step the old one down first (**Stop Being Orchestrator**, or `set-role` with
`agent` or `shell`).

### One place for the orchestrator, on the Mac

On the Mac, an orchestrator is shown only in its workspace's conversation
column, not also as a terminal under a worktree. (The phones still list it
among the worktree's terminals.) It's one pane: the column draws the
orchestrator and nothing else. A split there (⌃B %, ⌃B " or ⌃B c) opens a new
shell in the main checkout instead, beside the column, and **Changes** opens
the main checkout's changes the same way. Every other terminal in the main
checkout is listed under it in the sidebar and opens there.

If another terminal already shares the orchestrator's tmux window, such as a
shell split beside a Claude you later adopted, or a split made in the column
before this changed, the column draws the window whole, says so ("`name`
shares the orchestrator's window.") and offers **Move to Its Own Window**. That
click gives the other terminal a window of its own, so the orchestrator keeps
its window and focus. Opening that terminal from the main checkout moves it the
same way, and so does **Use as Orchestrator** for whatever shares the window of
the terminal it adopts. Nothing rearranges your windows unless you do one of
those.

## Splitting off a workspace

When one thread of work is crowding out the rest, ask the orchestrator to split
it off. It asks you for a name and a short task prefix (such as `bil`, so tasks
read `bil-9`), then:

1. creates the workspace, with a charter copied from Main's and edited down to
   this line of work;
2. moves the tasks, and the worktrees their agents work in;
3. writes a handoff on one of the moved tasks: open questions, your preferences
   for this work, what was tried and dropped;
4. starts the new workspace's orchestrator, pointed at that handoff.

The new orchestrator reads the handoff before anything else, so it starts with
what the old conversation knew. From then on, that work belongs to the new
workspace, and you talk to its orchestrator about it.

The orchestrator points the new one at its handoff with `--read`:

```
farcooler workspace create --repo overnight --name Billing --prefix bil
farcooler workspace start-orchestrator Billing --harness claude --read bil-3
```

## On the phone

The iPhone and Android apps have the same shape as the Mac, in each platform's
own style. They wait to hear from every runner (up to ten seconds), then open
on Needs You when something is waiting, and otherwise on the last workspace you
had open, with Needs You still one Back away. The iPhone differs in one way: it
reopens the screen you left, whether a workspace, a task or a worktree, even
with items waiting, and falls back to Needs You if that screen is gone. The
Mac and Android open on Needs You whenever something is waiting.

Needs You lists the waiting items, answerable in place, and then your
workspaces, grouped by runner and repository. A workspace has three views:

- **Orchestrator**: its conversation, as a terminal or as chat. A workspace
  without one offers **Start Orchestrator**.
- **Board**: the board as a sectioned list, with "N tasks are waiting on you"
  above it when any are. **New Task…** files a task by hand, with a title and
  optional details; it's offered when the runner lets the phone write to the
  board. Select a task to see its card and answer its decision, then go on to
  its **Agent**, its **Changes** or its **Worktree**. Back always returns to the
  task, and then to the board.
- **Worktrees**: the worktrees it owns, with their task keys. **New Worktree…**
  here makes one that belongs to this workspace. On the iPhone, swipe a
  worktree to **Hide** it; hidden ones are kept under **Hidden**, where the
  same swipe says **Unhide**.

A notification or widget that opens an agent lands on its workspace and task
first, so Back walks up from the agent to the task to the workspace. (An
orchestrator opens on its workspace's Orchestrator view, since it works no one
task.) The push a runner sends when the app is closed leads with the workspace's
name: "Billing · claude needs you", "Billing Orchestrator needs you", or
"Billing · bil-7 needs a decision" when the orchestrator asks you something.
Opening a decision push lands on its task. A banner the app posts itself leads
with the agent instead ("claude needs you"), with the place in its body.

On the watch, Needs You comes first. Permission asks can be answered there;
decisions and reviews say "Open on iPhone".

## When a runner goes quiet

Each runner tells the push relay it's alive every five minutes while it's
paired for notifications. When the relay hasn't heard from one for fifteen
minutes, the iPhone's widget and the watch's list and complication say "Lost
touch with" that runner (the widget's footer reads "lost touch with Studio"),
and stop saying that runner's agents are working. The Live Activity does the
same: its tail reads "+1 more · lost touch with Studio", and a card left with
nothing to show stops saying "Working". A blocked agent's question stays up.
The word comes from the relay, not from the phone's own clock, so a runner the
app reached directly a moment ago isn't reported.

A runner you unpair on purpose (`farcooler push forget`) tells the relay first,
and just disappears from the widget and watch. If that call fails, the runner
can read as lost touch for up to a day. The Mac and Android don't show this.

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

**Workspace**, because the sentence is about a line of work:

- `New Workspace…`, `Start Orchestrator`, `Show Charter`, `No orchestrator` — a
  workspace's board, charter and orchestrator, which there's one of per line of
  work rather than one per directory.
- `Move to Workspace` and `Unclaimed` — which worktrees a workspace owns.

A sentence that teaches the relationship needs both words: a workspace owns
worktrees; a worktree belongs to at most one workspace.

## One vocabulary everywhere

The CLI, the wire labels, and the code identifiers follow the same rule as the
copy. There is one user, and every surface updates together, so there is no
older script or older app to keep speaking the old words for.
`farcooler worktree` creates, lists, adopts, hides, reorders, removes and
assigns worktrees, and searches their files (`create`, `list`, `adopt`,
`branches`, `hide`, `unhide`, `reorder`, `assign`, `remove`, `file-search`).
`farcooler workspace` has `create`, `list`, `show`, `rename`, `set-prefix`,
`delete` and `start-orchestrator`. `farcooler task` works a board. The wire
methods for worktrees are `worktree.*`, and the call that lists worktrees git has
but Far Cooler has not adopted is `worktree.discover`.

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
