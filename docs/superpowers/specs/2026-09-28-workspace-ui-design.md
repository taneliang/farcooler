# The workspace UI

Date: 2026-09-28
Status: design, decided (rulings in §12); plan: `docs/superpowers/plans/2026-09-28-workspace-ui.md`
Card: follows ov-55 (the UI/UX review)
Builds on: [`2026-09-27-workspaces-as-workstreams-design.md`](2026-09-27-workspaces-as-workstreams-design.md)
Evidence: `.claude/agent/reports/workspaces-ux-map*.md` (four maps of `main` at 69b97934), re-checked against
`main` at fbcc57c4. Every claim below about today's code was read in the tree at fbcc57c4. Lines moved
since the maps were made, so the citations here are the current ones. Mac paths are relative to
`apps/macos/Sources/FarCooler/`, `AK/` is `apps/shared/AgentKit/Sources/AgentKit/`, `ios/` is
`apps/ios/FarCooler/`, and `android/` is `apps/android/app/src/main/java/com/farcooler/`.

## What the owner decided

These are binding. The rest of this document works them out.

1. **Direction A, with B's inbox.**
   - The Mac sidebar lists workspaces, per repository. Each workspace is one row, with its orchestrator's status
     and a needs-you count. A "Needs You" row sits at the top.
   - Selecting a workspace shows its navigator on the left, the board in three sections (the orchestrator, its
     tasks and its worktrees), with the orchestrator selected in the main area beside it (ov-92; ov-89 had the
     orchestrator fill the main area with the board as a sidebar on the right and a rail for the orchestrator,
     ov-85 had the board fill the detail beside that rail, and before that the conversation was a column
     beside the board).
   - Selecting a task, or a worktree, in the navigator shows it in the main area in the orchestrator's place,
     a task with its agent's terminal and changes beneath it. The orchestrator is kept, hidden, and comes back
     selected with ⌥⌘1, Esc or its row. The navigator doesn't move. See §4.3.
2. **A worktree is no longer a place of its own.** It's one click under a task, or under a workspace's
   disclosure of its worktrees, which also holds a shell in a branch that has no task. (The review called this
   disclosure "Helpers"; it's labeled "Worktrees", per ruling 9 in §12.)
3. **The board is the status-sectioned list**, ported from the iPhone, at every width.
   - Empty statuses are never hidden: an empty status is a collapsed header with a 0 count.
   - There was a kanban for a wide board, with a toggle in the header. The owner removed both (ov-83): the board
     almost always sits in a narrow column beside the conversation.
4. **The vocabulary stays:** workspace, orchestrator, board, worktree, task, agent. Nothing is renamed; the drift
   is fixed.
5. **The phones mirror A as first-class clients.**
   - The front door is the inbox.
   - Next come the workspaces, each with Orchestrator, Board and Worktrees.
   - iOS and Android get the same structure.

## Contents

1. [Vocabulary](#1-vocabulary)
2. [The "needs you" rollup](#2-the-needs-you-rollup)
3. [Task, agent and worktree links](#3-task-agent-and-worktree-links)
4. [The Mac window model](#4-the-mac-window-model)
5. [The responsive board](#5-the-responsive-board)
6. [The phones](#6-the-phones)
7. [Glance surfaces](#7-glance-surfaces-watch-widget-live-activity-push)
8. [Empty and first-run states](#8-empty-and-first-run-states)
9. [Migration](#9-migration)
10. [Slices](#10-slices)
11. [Risks](#11-risks)
12. [Rulings](#12-rulings)

---

## 1. Vocabulary

Six nouns, used the same way on every surface. The drift the maps found is listed with its fix. Code
identifiers aren't touched unless they surface in copy.

| Noun | Means | Today's drift | Fix |
|---|---|---|---|
| **workspace** | A workstream: a name, a task prefix, a board, a charter, at most one orchestrator, and the worktrees it owns | The phones never say the word. Headings show only the name (`ios/ShellOverview.swift:1141-1169`; `android/ui/FleetScreen.kt:986-1023`) | Section headings in lists read "Workspaces". Empty states and menus say "workspace" |
| **orchestrator** | The workspace's lead agent | "The manager" as a note byline (`AK/TaskBoardModel.swift:731`). Android's row shows the program name, and only the tab says "Orchestrator" (`android/model/FleetLayout.kt:100-101`) | "Orchestrator" everywhere in copy. The byline becomes "Orchestrator". `manager` stays the actor and skill name, which users don't see |
| **board** | A workspace's tasks, by status | "Main board" on Android's row but "Main" on the board it opens (`android/ui/BoardScreen.kt:116`). "Show this project's board" (`Shortcuts.swift:115`) | "Board" as a tab and column title, and "<Workspace> Board" where it stands alone. "Project" goes |
| **task** | A card on a board, with a key such as `bil-9` | "Quick Task" makes a worktree and an agent but files no task (`ios/QuickTask.swift`, `android/ui/FleetScreen.kt:332`). `PaletteAction.newTask` is the palette's "New Worktree…" (`PaletteIndex.swift:16`) | "Quick Task" becomes "New Worktree…", the Mac's word for the same action (`Commands.swift:89-90`). The palette case is renamed in code |
| **agent** | A coding agent (claude, codex, cursor) running in a terminal | The same pane is called a terminal, pane, tab or session | "Agent" when a terminal runs an agent; "terminal" when it runs a shell. "Pane", "tab" and "session" leave user-facing copy (VoiceOver included) |
| **worktree** | A directory and branch | "Primary checkout" (`SidebarViews.swift:208`; `android/ui/NeedsYouScreen.kt:404`) vs the CLI's "main checkout". "Diff" on iOS (`ios/ShellScreen.swift:294`) vs "Changes" on the Mac (`ContentView.swift:189`) | "Main checkout" in every app. "Changes" on every platform |

Two words are not in the six but appear in copy:
- **Repository** replaces "Project" (`QuickCreate.swift:160`, `ios/TaskComposer.swift:91-92`, `ContentView.swift:2693`).
- **Runner** stays.

**"Worktrees" labels the disclosure** (and the phone tab) that holds a workspace's worktrees, so no new noun is
introduced. Rows inside it are worktrees; their terminals are agents or terminals.

**Copy rules.** These follow Apple's conventions:
- title-case buttons and menu items ("Start Orchestrator", "Open Worktree");
- sentence-case descriptions and empty states;
- contractions;
- no raw runner errors in the UI.

---

## 2. The "needs you" rollup

### 2.1 Why one definition

The surfaces disagree today:

| Surface | Counts today |
|---|---|
| Mac header badge and ⌃⌘N | Terminals whose agent is Blocked or Done (`ContentView.swift:2656-2663`; `wants_attention` in `crates/core/src/activity.rs:891-892`) |
| Mac Board row | Tasks in Needs Decision (`SidebarViews.swift:1278`) |
| Android front door | Blocked and finished agents, plus unread diffs, grouped by worktree (`android/model/NeedsYou.kt:182`), then Board rows beneath |
| iOS | No list. The inbox was deleted in 8b08c1f5 |
| Live Activity | `blocked`, `review` and `working` agent counts that the relay sums over every row (`services/relay/src/push.ts:269-285`) |
| Widget and watch | Agent rows only, named by program ("claude") |

The result is that none of these numbers agree for the same fleet at the same moment.

### 2.2 The definition

An **item** is one thing a person has to act on. It is one of four kinds, listed in rank order:

| Kind | When | Its subject | Ends when |
|---|---|---|---|
| **ask** | An agent is waiting on a permission that has an id and options, so it can be answered away from its terminal. Two sources: a claude TUI's held `PermissionRequest` (ov-14; `crates/daemon/src/hook_asks.rs:1-22`, ids prefixed `hook-ask-`, `:37`), and an ACP chat pane's `AgentEvent::Permission` | The terminal, plus its task when the terminal has one | The ask settles (`hook_asks.rs` `settle`, or the chat's `Resolved`) |
| **blocked** | An agent is Blocked and has no answerable ask: a codex or cursor TUI, a trust gate, or a question read off the screen. Also an agent whose last turn failed (`Terminal.turn_failed` with activity Done). A process that exited badly (`exit_wants_attention`, `activity.rs:905`) is **not** an item: nothing marks an exit as seen (`activity::seen` changes only Done), so it would stay until the terminal is removed. It keeps its `✗` glyph | The terminal, plus its task | Its activity leaves Blocked, or the failed turn is seen (`terminal.seen` turns Done into Idle, `watch.rs:3172-3184`) |
| **decision** | A task is in Needs Decision (`TASK_STATUS_NEEDS_DECISION`, `proto/farcooler.proto:2602`). Its question is the latest `QUESTION` note (`:2624`) | The task | An `ANSWER` note later than that question, **or** the task leaving Needs Decision. An answer only appends a note (`task_ops::note`, `crates/daemon/src/task_ops.rs:500-522`): nothing moves the status back, and the orchestrator stays in charge of the card (ruling 2's spirit). The task stays in Needs Decision on the board until it moves it |
| **review** | A task is In Review (`TASK_STATUS_IN_REVIEW`, `proto/farcooler.proto:2604`). Nothing else: a Done agent and a worktree with unread changes are not items (ruling 1) | The task | The task leaves In Review |

**Rules on top:**
- **One item per subject.** When one subject has several signals, the most urgent kind wins, and the others go on
  the item as `also`. For example, a task in Needs Decision whose agent also holds an ask is one item of kind
  **ask**, with `also: [decision]`.
  - A signal whose terminal has a `task_id` has the task as its subject.
  - An orchestrator is never a task's agent, so its items are always about its own terminal.
- **Ranking.** Every item has a `rank` on the same scale and rule as `Terminal.rank` (`proto/farcooler.proto`,
  field 29): a tier per kind, then the oldest first within a tier. The tiers are ask, then blocked, then decision,
  then review.
  - Ranks are durations, not clock readings, so two runners' items merge by rank without comparing clocks.
    `android/model/NeedsYou.kt:36-52` already relies on this property.
- **The count is the number of items**, not signals and not agents. A workspace's count is the number of its
  items. The Needs You row, the watch complication, the widget and the Live Activity header all show this one
  number.
- **Which workspace an item belongs to:**
  - the task's `workspace_id`;
  - failing that, the terminal's `workspace_id` (`proto/farcooler.proto:1178`);
  - failing that, the worktree's owner;
  - failing all three, none. An item with no workspace is counted under its repository's Unclaimed group, and in
    the Needs You total.
- **Hidden worktrees.** Their items still count. Android drops them today (`android/model/NeedsYou.kt:190`), which
  would hide an orchestrator whose main checkout is hidden.
- **Unread diffs and finished agents are not items** (ruling 1). A worktree with unread changes keeps its dot on
  its row, and a Done agent keeps its `✓` glyph and its existing Done notification. So ⌃⌘N, which walks items, no
  longer stops at a finished agent; it did before (`wants_attention` counts Done, `activity.rs:891-892`).
- **An orchestrator's finished turn is not an item** (ruling 10). It's an unread dot on its workspace row and in the
  conversation column's header, cleared when its terminal is seen. Its asks and blocks are items as usual.

### 2.3 Where it's computed: the daemon

**Each runner's daemon computes its own items.** Each client merges the lists from all its runners by rank.

**Why the daemon, and not `crates/client` or AgentKit:**

1. **Only the daemon holds all four facts:**
   - held asks, with their ids and options (`hook_asks.rs`);
   - chat permissions (`AgentSupervisor`);
   - agent activity, which is derived on the host and never by a client (`proto/farcooler.proto:853-857`);
   - the tasks.

   A client can't see the ids and options of an ask without subscribing to that terminal's agent stream. That is
   why the glance surfaces keep their own copy today (`AK/GlancePermissions.swift:1-50`).
2. **`crates/client` doesn't reach the Mac.**
   - The Mac talks to runners by running the CLI (`DaemonClient.swift:50-57`).
   - It links the client core only for the enrollment ceremony (`Sources/CFarCoolerClient/module.modulemap`
     comment).
   - A definition in the client core would therefore reach two apps and not the third.
3. **AgentKit doesn't reach Android.** A definition in AgentKit would need a Kotlin copy, and two copies of a rule
   are what drifted here in the first place.
4. **A suspended phone gets news only by push**, and pushes are composed on the daemon (`crates/daemon/src/watch.rs:338-354`)
   and summed by the relay. A lock-screen count can agree with the app only if both come from the daemon.
5. **There's precedent.**
   - `changes.inbox` is a daemon-computed rollup that clients poll (`crates/daemon/src/review_ops.rs:341-399`).
   - `rank` is computed "here, beside `activity`, so an Island showing one agent and a sidebar showing twelve
     agree".

**What the shared client core does:** it renders the JSON projection, once.
- `crates/client` gains `needs_you_json`, shared by `farcooler needs-you --json` (which the Mac reads) and
  `Session::needs_you` (which the phones read through the FFI).
- This follows `changes_json::inbox_json`, which is already shared by `farcooler changes inbox --json` and
  `Session::changes_inbox` (`crates/client/src/session.rs:1636-1644`).

**What each app does:**
- It decodes the one JSON shape.
- It merges runners by rank.
- It renders.

The decode and merge live in AgentKit for the Mac, iPhone, watch, widget and Live Activity, and in Kotlin for
Android. Both are pinned by one fixture file that both test suites read, as the fleet decode tests already do
(`AgentKitTests/FleetDecodeTests.swift`, `FleetDecodeTest.kt`).

### 2.4 Wire

**New RPC `needs_you.list`,** Read scope, with the content gated by scope in the converter, as `worktree_path` is.
- Read, like `changes.inbox` (`crates/daemon/src/rpc.rs:414`): the shape of the work is metadata.
- **Below Control scope** (ruled): an item carries its `id`, `kind`, `also`, `rank`, `since`, workspace, task and
  terminal, and nothing else. `question` becomes a fixed sentence for the kind ("claude is asking to use a tool",
  "claude needs you", "Needs a decision", "Ready for review"), and `detail`, `ask_id`, `actions` and the worktree's
  path are left out. **Why:** an ask's option names carry the raw command or file path (`claude_tool_title`,
  `crates/agent-core/src/permission.rs:26-31, 80-95`), and that text travels today only on the agent channel, which
  is Control (`rpc.rs:388-394`). A decision's question is a board note, which Read may already see through
  `task.get`; it's blanked anyway, so one rule covers every kind.
- At Control, `question` is the ask's allow-option name ("Allow touch x"; there's no separate title), or
  `blocked_question`, which is already redacted on the host, or the `QUESTION` note's body.

**New event `needs_you_changed`** (Empty, `Event` tag 24), so clients re-read instead of polling. It fires on:
- a terminal's activity change;
- a held ask's `hold` or `settle`;
- a chat's `Permission` or `Resolved`;
- `task_changed` into or out of Needs Decision or In Review;
- a `QUESTION` or `ANSWER` note on a task already in Needs Decision (the status doesn't move, but the item does);
- `terminal.seen`.

`Watcher::announce_fleet_changed` (`watch.rs:3217`) sends one event per call and doesn't coalesce. This event is
debounced in the watcher instead: at most one per 250 ms, trailing edge.

**Fresh tags,** checked against `proto/farcooler.proto` at ddfb24d1:
- `Event.needs_you_changed = 24` (the last is `events_missed = 23`, with no gaps);
- `Result.needs_you_list = 45` (the last is 44; the unused 27-31 stay unused);
- `Worktree.open_tasks = 14` (the last is `foreign_writer_workspace_ids = 13`).

`needs_you.list` takes no payload, like `terminal.list`.

**New capability `needs_you`,** a constant beside `workstreams` (`crates/protocol/src/lib.rs:240`) and an entry in `capability::ALL` (`:394-399`), without which the daemon doesn't advertise it.

**The item** (proto sketch; tags are fresh):

```
message NeedsYouItem {
  string id = 1;                       // stable across reads: "ask:<ask id>", "blocked:<terminal>",
                                       // "decision:<task>" or "review:<task>"
  NeedsYouKind kind = 2;               // NEEDS_YOU_KIND_ASK, _BLOCKED, _DECISION, _REVIEW (UNSPECIFIED = 0)
  repeated NeedsYouKind also = 3;      // the subject's other, less urgent signals
  uint32 rank = 4;
  google.protobuf.Timestamp since = 5;
  bytes workspace_id = 6;              // empty for none, as Task.workspace_id is; plus its name, below
  string workspace_name = 7;
  bytes repository_id = 8;
  optional TaskRef task = 9;           // id, key, title, status
  optional TerminalRef terminal = 10;  // id, worktree_id, label, role, pane_mode, chat_capable
  optional WorktreeRef worktree = 11;  // id, name, branch, insertions, deletions
  string question = 12;                // redacted, one row wide: the ask's title, blocked_question,
                                       // the QUESTION note, or "Ready for review"
  optional string detail = 13;         // an ask's command; a review's "+18 −40"
  optional string ask_id = 14;         // what terminal.agent_answer takes
  repeated NeedsYouAction actions = 15;
}
message NeedsYouAction {
  string id = 1;        // an ask option id ("allow", "deny"), a decision option's text, or "open"/"seen"
  string title = 2;     // "Allow", "Deny", or the option as written
  bool destructive = 3;
  bool primary = 4;
}
```

### 2.5 What each item carries, and how it's answered in place

| Kind | Buttons (Mac, phone) | Sends | Where Open lands |
|---|---|---|---|
| ask | **Deny**, **Allow** (the ask's own options, in its order) | `terminal.agent_answer(terminal, ask_id, option)`: the existing path (`rpc.rs:2090`), which refuses a stale or second answer | The task, drilled into (or the orchestrator's conversation) |
| blocked | **Open** | nothing; the answer is typed in the terminal | The same |
| decision | The question's options, when the note has some (at most three as buttons; more go in a menu). **Answer…** when there are none | `task.note` of kind `ANSWER` (Control scope, `rpc.rs:428-432`), with the option's text, as the user | The task, with its card expanded |
| review | **Review** only. The inbox opens a review; it never approves one (ruling 2) | nothing | The task, drilled into, with its changes |

**Read-scoped clients** see the items without buttons, except Open. That is the same rule the board uses.

**Mid-answer.** A button that has sent shows a spinner in place. When the answer succeeds, the item leaves on the
next `needs_you_changed`. When it's refused, the item stays, with one line:
- "Someone already answered this." for `NotHeld`;
- "Couldn't reach claude. Try again." for `NotDelivered`.

Today both refusals are the same `ResourceConflict` with no `what` (`rpc.rs:2105-2110`), so no client can tell them
apart. `terminal.agent_answer` sets `what` to `not_held` or `not_delivered`, and the CLI's error JSON carries it.

This fixes the ov-54 review finding that a failed answer cleared the card silently.

### 2.6 Older runners

A runner without `needs_you` still contributes items, derived in the app from what it already sends:
- Blocked agents, from `Terminal.activity`, become **blocked** items. No asks, decisions or reviews are derived.
- Their rank is re-tiered into the blocked tier. `Terminal.rank`'s tier 0 is Blocked, while the item scale's tier 0
  is ask, so a derived item copied as is would outrank a real ask on another runner.
- Its section in Needs You says: "Update Far Cooler on <runner> to see decisions and asks here."
- Nothing is guessed.

---

## 3. Task, agent and worktree links

### 3.1 What exists

| Link | Data | Used by |
|---|---|---|
| terminal → task | `Terminal.task_id` (`proto/farcooler.proto:1168-1173`), set by `farcooler task dispatch` and `terminal new --task`. Decoded by the Mac (`Model.swift:408-415`), AgentKit (`AK/CoreModel.swift`) and Android (`android/model/Model.kt:343`) | Only the board's "which agent is on this card" rule (`AK/TaskBoardAgents.swift:10-58`; Android `TaskBoard.kt:168-173`). **No pane, pane bar or Changes view names its task on any platform** |
| task → agent | Derived: the live panes whose `task_id` matches and which run an agent (`TaskAgentLink.isWorking`, `AK/TaskBoardAgents.swift:55-58`) | Board cards' "Go to Agent" (Mac `TaskBoard.swift:742-836`), the iOS Agent pill, and the Android chip |
| task → worktree | `Task.worktree_id` (`proto/farcooler.proto:2656-2658`), set by dispatch through a second `task.update` after the pane opens (`crates/cli/src/tasks.rs:2588-2597`) | **Nobody.** Decoded by AgentKit (`AK/TaskBoardModel.swift:572, 608`) and Android (`android/model/TaskBoard.kt:85, 290`), and never read by a view. A task in review with no live agent can't reach its changes on any platform |
| terminal → workspace | `Terminal.workspace_id` and `role` (`proto/farcooler.proto:1174-1180`) | The Mac sidebar's orchestrator rows; ov-60's notifications |
| workspace → orchestrator | `Workspace.orchestrator_terminal_id` (`proto/farcooler.proto:2970`) | The sidebars' orchestrator rows |
| worktree → tasks | **None.** Answering "which task is this worktree for" needs every board loaded, and the phones read boards lazily (`AK/RunnerBoards.swift:125-172`) | — |

### 3.2 What's added

1. **A worktree knows its tasks.**
   - `Worktree` gains `repeated TaskRef open_tasks = 14`: `{id, key, title, status}` for every task whose
     `worktree_id` is this worktree and whose status is not Done or Cancelled. Not `tasks`: the CLI's worktree JSON
     already has `"task"`, the worktree's own name.
   - It's filled where `WorktreeView` is built, since `wire::worktree` (`wire.rs:189`) has no store. It appears in
     both projections (the CLI's `worktree list --json`, which the Mac reads, and `Session::fleet`, which the
     phones read), and in all three decoders.
   - `worktree_changed` fires for every change to it: `task.update` moving `worktree_id` (both lanes) or changing
     the title; `task.create` naming a worktree; and `task.set_status` into or out of Done or Cancelled.
   - The daemon fills it in `worktree.list`, and `task_changed` triggers `worktree_changed` for the old and new
     lanes.
   - This is what lets a Worktrees row read "fc-3-webhooks · bil-9", and a pane's header name its task, without a
     board read.
2. **A pane shows its task.** Its task is:
   - `Terminal.task_id` when it's set;
   - otherwise its worktree's task, when the worktree has exactly one.
   - never, for an orchestrator pane: it leads the workspace and works no one task (ruled during ov-55 1E).

   This is a rule, `TaskLink.task(of:in:)`, stated once in AgentKit and once in Kotlin against one fixture. It
   draws as a chip in the pane's header: "bil-9 Invoice PDF export". The chip opens the task.
   - Mac: the task view's work line, and a worktree layout's `GroupBar`.
   - iOS: the pane bar.
   - Android: the worktree screen's top bar.
3. **A task reaches its worktree and changes** through `worktree_id`, with or without a live agent.
   - Mac: **Open Worktree** in the task view.
   - Phones: **Changes** and **Worktree** rows on the task screen.
   - Android's decoded-but-unused `worktreeId` gets its first reader here.
4. **Back from a task's agent to the task.** The Mac keeps the task open while you're in its agent. The
   phones push the agent onto the workspace's stack, so Back returns to the task. This removes the iOS dead end
   where "Go to Agent" closes the board (`ios/ShellScreen.swift:1150-1166`, `:2259`).

**Not changed:** `task_id` stays explicit. The daemon doesn't infer a pane's task from where it works, because a
person's shell in a task's worktree isn't that task's agent. The worktree fallback in item 2 is a display rule
only; it never feeds dispatch's "one agent per task" check.

---

## 4. The Mac window model

### 4.1 Today

- **One window**, `WindowGroup { ContentView() }`, with a minimum size of 600×400 (`FarCoolerApp.swift:28`).
- **A two-column `NavigationSplitView`** (`ContentView.swift:162`). The sidebar is 268–440 pt wide, ideal 320
  (`:1205`).
- **The detail shows one `Selection`** (`:138-146`): `.worktree`, `.terminal`, or `.board`.
- **The detail switch** (`:1813-1939`) draws one of three things:
  - a tiled tmux layout;
  - a `WorktreeDetail`;
  - the full-width `TaskBoardView`.
- **The board and the orchestrator replace each other.** The orchestrator row selects a `.terminal`
  (`:942-957`), and the Board row selects `.board` (`:921-938`).

### 4.2 The new selection

```swift
enum Selection: Hashable {
    case needsYou
    case workspace(host: String, workspace: String, focus: Focus?)
    /// A worktree no workspace owns, or any worktree on a runner without `workstreams`.
    case looseWorktree(host: String, worktree: String, terminal: String?)
}
enum Focus: Hashable {
    case task(String)                                    // task id
    case worktree(String, terminal: String?)             // a worktree, from a task's Open Worktree or the Worktrees disclosure
}
```

**Where the old cases go:**

| Old | New |
|---|---|
| `.board(h, w)` | `.workspace(h, w, focus: nil)` |
| `.terminal(h, wt, t)`, where `t` is an orchestrator | `.workspace(h, t.workspace, focus: nil)` |
| `.terminal(h, wt, t)`, where `t` has a task (per §3.2's rule) | `.workspace(h, task.workspace, focus: .task(id))` |
| `.terminal(h, wt, t)`, any other | `.workspace(h, owner(wt), focus: .worktree(wt, t))`, or `.looseWorktree` when unclaimed |
| `.worktree(h, wt)` | The same, with `terminal: nil` |

**A runner without `workstreams`** keeps working. Each repository is one implicit workspace
(`WorkspaceSummary.implicit`, which exists): it has a board and Worktrees, and no orchestrator column.

### 4.3 The workspace view

**The board is the workspace's navigator, on the left; the main area shows what's selected in it** (ov-92,
superseding ov-89's orchestrator-filling main area, board sidebar on the right and orchestrator rail). The
owner, 2 Oct, of the rail: "the orchestrator's bar color makes it blend in with the sidebar and the area to the
right… what about making the task list basically a second level sidebar almost, then have the orchestrator be
somewhere in the board basically? it already has a worktrees section (which should probably be separated from
the sections for tasks actually) so having the orchestrator in there might also work well."

```
┌ NAVIGATOR ─────────────┬ MAIN ───────────────────────────────┐
│ ORCHESTRATOR           │                                      │
│ ◉ claude · Working     │   whatever is selected on the left:  │
│   Running the Mac      │   the orchestrator by default,       │
│   tests for ov-91      │   or a task, or a worktree           │
│   3 tasks in progress  │                                      │
│ ────────────────────── │                                      │
│ TASKS                  │                                      │
│  Since Last Visit …    │                                      │
│  Needs Decision 1 bil-7│                                      │
│  In Progress 2   bil-3 │                                      │
│  To Do · Done · …      │                                      │
│ ────────────────────── │                                      │
│ WORKTREES              │                                      │
│  ⎇ main (main checkout)│                                      │
│  ⎇ scratch             │                                      │
│  + New Worktree…       │                                      │
└────────────────────────┴──────────────────────────────────────┘
```

- **The navigator** is a source list on the left of the main area, the Mac's convention; the optional sidebar,
  when shown, is to its left. 280 pt until its trailing edge is dragged, then the width it was dropped at
  (`workspace.navigatorWidth`, per Mac), never under 240 pt, and never so wide that the main area loses its 58
  columns. Below that, the main area gives way and the navigator keeps its minimum. It's on the board column's
  grid (ov-83), under the shared column header (below), and keeps its scroll.
- **Three sections**, each under its own header (small capitals, secondary, at column A) and a divider apart:
  - **Orchestrator:** one row (`OrchestratorRowView`). Its harness (claude, codex); its state as an icon and a
    word, from its own pane alone (`OrchestratorRow.state`): Working (a spinner), Starting (a spinner), Needs
    You (the accent dot: it's blocked), Done (the amber dot: a turn nobody has seen), Idle, Stopped. A
    **now-doing** line, one line, truncated, secondary (`OrchestratorRow.nowDoing`), from what the runner
    already sends for the pane: blocked, the question; working, its hook-reported activity or plan position
    (`line`), else the last thing it said (`said`); finished or idle, the last thing it said, else "Idle since
    3:42 PM" (`activitySince`). No daemon change. Then "3 tasks in progress", counted off the board. With no
    orchestrator, the row reads **No Orchestrator**, with **Start Orchestrator** ▸ and, for a claude running in
    a shell here, **Use as Orchestrator…** ▸. A repository's implicit board, which can't have one, has no
    Orchestrator section.
  - **Tasks:** the status groups, as before (§5), under the Since Last Visit summary.
  - **Worktrees:** the loose ones (ov-86), now a section of their own rather than a group among the statuses,
    their hidden ones collapsed under them, and **New Worktree…** as the section's last row. A task's worktree
    is named on its task's row, never here; the main checkout is listed without the orchestrators seated in it.
- **The main area shows exactly one thing, what's selected:** the orchestrator (`focus` nil, the default), a
  task, or a worktree. A task or a worktree is under the jump bar (Workspace › Task, Workspace › Task ›
  Worktree, or Workspace › Worktree, with the worktree menu at its end, and its ×). A workspace without an
  orchestrator draws the conversation's empty state, "No Orchestrator", with **Start Orchestrator** and **Use a
  Running Terminal…** (ov-63). Only a repository's implicit board on a runner without `workstreams` says "No
  Orchestrator" / "Update Far Cooler on this runner to use an orchestrator here." (`WorkspaceMain`).
- **The orchestrator is mounted once and kept** at the main area's width, hidden (not destroyed) while a task
  or a worktree is selected, so going back to it is instant and nothing in it re-wraps: its tmux window is
  resized only when the window's width or the navigator's changes. Hidden, it takes no click, no keyboard
  (`outOfSight`, disabled) and isn't seen or watched; selected, it's on screen by the selection's rule
  (`WorkspaceScreen.visible`, `KeptOrchestrator`).
- **Selecting** cross-fades the main area on the shared spring, interruptibly: what takes a click and the
  keyboard follows the selection at once, never the fade.
- **Keyboard:** ⌥⌘1 selects the orchestrator and gives it the keyboard; ⌥⌘2 gives the navigator the keyboard
  (leaving Focus); ⌥⌘3 gives the main area the keyboard, whatever it shows. ↑/↓ while the navigator has the
  keyboard walk the whole navigator, the orchestrator, then the tasks as the list shows them, then the
  worktrees (`Navigator.items`, `Navigator.step`), held at either end. Return goes into what's selected. **Esc**
  in the main area, when no terminal or field has the keyboard, goes back to the orchestrator (past the task a
  worktree was opened from); ⌃⌘← goes up one level along the breadcrumb. ⌃⌘↑/↓, ⌘0 and ⌘1–9 are unchanged.
- **Glancing:** a held arrow walks the navigator: the selection and a task's header and text change at once on
  every step, and what costs something (its terminal mounted, its `task show` read, the changes read) waits
  until the selection has stayed put for 150 ms (`WorkspaceMotion.settle`), so passing a row costs nothing. The
  task leaving fades in 0.08 s while the next fades in on the spring, so two records never overprint for long
  (ov-65's O2 still holds: no task ever draws another's record). The navigator keeps the keyboard throughout: a
  click on a row, ⌥⌘2 and a close all give it to the navigator.
- **Where the keyboard goes** for each command is one pure function, `WorkspaceNavigation.boardStep`
  (`BoardKeyboardTests`): the navigator never holds the keyboard where it isn't drawn, and Focus with nothing
  open changes nothing.
- **A worktree opened** from a task (Open Worktree) is shown in the main area, as is one opened from the
  sidebar. A **loose worktree** (one no workspace claims, or any on a runner without `workstreams`) is shown
  there too, beside its repository's navigator (Main's, or the implicit one), with that board's orchestrator
  kept behind it; an implicit board has none.
- **Focus** (⌃⌘↩) shows what's selected alone: no navigator, and a task's terminals and changes at full height
  without its text, in the same views. ⌃⌘↩ again, ⌥⌘2, or Esc when no terminal has the keyboard, puts the
  navigator back.
- **Column headers:** every column's header (the navigator's, the orchestrator's, the jump bar over a task or
  a worktree) is drawn through one modifier, `columnHeader()`: 32 pt (`ColumnHeader.height`) over the one
  divider, so their bottom edges are one straight line across the window (`ColumnHeaderTests`). The owner, 2
  Oct: the board's "Main" header stood taller than the orchestrator's.
- **The jump bar** is Xcode's: every segment at the header's one size (12 pt) on one baseline; the ancestors
  secondary and regular, the current one primary; one tertiary chevron for every separator and for the
  worktree menu's ⌄; the worktree's ⎇ at the text's size; a long task title truncated in its middle, never
  resizing the other segments (`JumpBar`, `DrillBreadcrumb.pieces`). The owner, 2 Oct: "the text is different
  font sizes etc it should be consistent size at least."
- `WorkspaceColumns.layout(opened:hasConversation:hasBoard:focused:)` and `frames(width:arrangement:navigator:)`
  decide all of this as values.

| Part | Holds | Min | Default |
|---|---|---|---|
| **Navigator** | The board (§5) in three sections, Orchestrator, Tasks and Worktrees, on the left | 240 pt | 280 pt |
| **Main area** | What's selected: the orchestrator, the task (§4.4) or the worktree | 58 columns: 489 pt, while the navigator can give way | the rest |

**Wireframe before ov-92, the orchestrator filling the workspace** (ov-89; superseded: the board is the
navigator on the left, and the orchestrator a row in it):

```
Workspace (nothing open)                          A ticket or worktree open
┌──────────────────────────────┬──────────┐      ┌─┬─────────────────────────┬──────────┐
│ ORCHESTRATOR                 │ BOARD    │      │O│ bil-3 Invoice PDF export│ BOARD    │
│ (main content, fills)        │ Needs you│      │r│ Intent · Acceptance     │ Needs you│
│                              │ ● bil-7  │      │c│ Activity (newest first) │   bil-7  │
│                              │ In Prog. │      │h│ ── worktree bil-3 ──    │ ▸ bil-3 ◀│
│                              │   bil-3  │      │ │ [ terminal ]  [Changes] │   bil-9  │
│                              │ Worktrees│      │ │                         │ Worktrees│
└──────────────────────────────┴──────────┘      └─┴─────────────────────────┴──────────┘
```

**Where the minimums come from.** Measured on 2026-09-28 (ov-55, 2A) against the app's own code, not estimated:
- **A cell is 7.727 pt wide** at the default terminal font (SF Mono, 12.5 pt; `TerminalMetrics.cell`), not 7.5.
- **A column of `W` pt holds `floor((W − 40) / 7.727)` terminal columns.** The 40 pt is the canvas inset
  (`Pane.inset`, 10 each side) plus the terminal's own padding (`TerminalMetrics.padding`, 10 each side), as
  `TileGeometry.viewport` counts it. Chat mode reports the same grid (`AgentSurface.report`). So 400 pt is 46
  columns, not 50, and 500 pt is 59, not 64.
- **The conversation's 48 columns.** The real screens in `crates/core/captures` set it:
  - Claude Code reflows cleanly down to 40 columns (`claude-background-agent-main-idle-40col.txt`). Only its mode
    line truncates.
  - At 48, every fixed line an orchestrator shows on its own screen fits, except that claude's AskUserQuestion
    footer (`Enter to select · ↑/↓ to navigate · Esc to cancel`, 49 columns) wraps by one.
  - 48 is also what an iPhone shows. SF Mono 13 on a 393–402 pt screen, less 12 pt of padding, is 47–48
    columns. Narrowing the orchestrator to its minimum barely changes a phone that's showing it. See R2.
  - In chat mode, 412 pt leaves the transcript about 60 characters of 13 pt body text a line, and the composer
    shows one selector inline and folds the rest into its `⋯` menu (`AgentComposer.inlineCount`).
- **The task's 58 columns** (measured for the old third column; now what the main area keeps as the navigator is dragged wider, `WorkspaceColumns.openedColumns`). That's the width at which no fixed line of a permission prompt wraps, for all
  three harnesses:

  | Harness | Widest fixed line in its permission prompt | Columns, with its indent |
  |---|---|---|
  | claude | `2. Yes, allow all edits during this session (shift+tab)` | 58 |
  | codex | `3. No, and tell Codex what to do differently (esc)` | 52 (its banner box is 56) |
  | cursor | `Skip & tell the agent what to do instead (esc or n)` | 55 |

  Lines that carry a command or a path wrap at any width, and do no harm when they do. Claude 2.1.283's
  `3. Yes, and switch to auto mode · auto mode handles these prompts for you` (76) and the tip above it (82) wrap
  once.
- **Changes never wraps a line.** `ChangesPane` scrolls a long line sideways (`contentWidth`). Below `wideEnough`
  (620 pt) it has no file column, and a diff `D` pt wide shows `floor((D − 69) / 7.727) − 2` characters before
  scrolling: 52 at 489 pt, 80 at 703 pt and 100 at 858 pt. From 620 pt up, a file column takes
  `min(280, max(220, W × 0.2))` of it, plus a 1 pt divider. So widening a task column past 620 pt narrows its diff: 69 characters at 619 pt, 40 at
  620, and 69 again only at 839 pt. 2D has to decide whether the task column accepts that cliff or keeps
  `ChangesPane` compact below about 840 pt.
- **The board's 280 pt** was one kanban column's outer width: the 260 pt card that held a task, plus its 10 pt
  padding each side. The kanban is gone (ov-83). As the navigator (ov-92) it's 280 pt by default and 240 pt at
  the least, which still holds a list card's key, a short title and its agent pill, and the orchestrator's
  state.
- **The navigator's divider costs 1 pt.** There's no `HSplitView` since ov-85 (it never collapsed a child on
  its own, only overflowed and clipped): the parts are placed from `WorkspaceColumns.frames`.
- **The minimums are in columns.** At a larger font they're wider in points: at 13 pt, the conversation's 48
  columns are 426 pt. `navigatorWidth(_:width:cell:)` takes the cell width, and its tests use the default font.

**When the main area can't keep its 58 columns** (`W < 240 + 1 + 489 = 730`, `W` the detail's width), the
navigator stays at its 240 pt minimum and the main area gives way; Focus (⌃⌘↩) gives it the whole detail. The
window's minimum is 600 pt, so the detail can be 352 pt; there, the main area is 111 pt.

At a 13-inch laptop's widths (the detail is the window less the sidebar when it's out, 248 pt: 1222 pt full
screen on an M2 Air, 1192 pt on an M1, 1032 pt in a 1280 pt window), the main area gets the detail less a
281 pt navigator: 941, 911 or 751 pt, the same for the orchestrator, a task or a worktree.

The sidebar narrows to min 220, ideal 248, max 360 (from 268/320/440, `ContentView.swift:1205`). It has to: at
today's ideal of 320, a full-screen 1470 pt window leaves a 1150 pt detail, and all three columns don't fit even
there. Its rows are now workspaces, and a worktree row shows up only under Worktrees.
- At 248 pt, a workspace row has about 175 pt for its name.
- A worktree row two levels in (at 50 pt) has 184 pt for everything. After `+42 −7` (38 pt), its gaps and its
  dot, that's about 128 pt for the name and task key: `fc-3-webhooks · bil-9` (136 pt) loses its last characters.
- At the 220 pt minimum, the same row has about 100 pt, which is `fc-3-webhooks` and no key. Names truncate, as
  they do today, so the narrower sidebar still works.

**Wireframe before ov-89, a task open beside the board** (ov-85; superseded: the board is now the sidebar on
the right, and the task takes the main area):

```
┌ Sidebar ──────────┬──┬ Billing ──────────── + ↻ ┬ Billing › bil-3 Tax rounding on credit notes ──── × ┐
│ ◉ Needs You     3 │O │ ▾ To Do              2   │ bil-3 Tax rounding on credit notes    In Progress ▾ │
│                   │r │   bil-2 Webhook retries  │ Intent  Round tax per line, not per note.           │
│ shop              │c │ ▾ In Progress        2   │ Record  …                                           │
│   Main        ●  1│h │ ▸ bil-3 Tax rounding ◀── │ ─ Worktree tax-rounding · claude working ────────── │
│ ▸ Billing  ●  2   │  │   bil-1 Invoice PDF      │ [ terminal ]                          [ Changes ]   │
└───────────────────┴──┴──────── 300 pt ──────────┴─────────────────────────────────────────────────────┘
```

**Wireframe before ov-85, drilled into a task** (superseded: the board is now the list beside the task):

```
┌ Sidebar ──────────┬ ‹ Billing › bil-9 Invoice PDF export ─────────────────────────────────────────┐
│ ◉ Needs You     3 │● │ bil-9 Invoice PDF export                                   In Progress ▾ │
│                   │• │ Intent  Customers can download any invoice as a PDF.                     │
│ overnight         │  │ Acceptance  ◉ renders line items  ○ matches the HTML  ○ under 1 s         │
│   Main        ●  1│  │ Record  Plan · agent · 2 h ago  …                                        │
│ ▸ Billing  ●  2   │  ├──────────────────────────────────────────────────────────────────────────┤
│   Relay   ◌       │  │ ⑂ Worktree fc-3-webhooks  2 terminals      agent · claude  Open Worktree │
│ ＋ New Workspace… │  │ ❯ writing render_invoice()…                                              │
│                   │  │ ── Changes  +42 −7 ──────────────────────────────────────────────────── │
│                   │› │ + fn render_invoice(…)                                                   │
└───────────────────┴──┴──────────────────────────────────────────────────────────────────────────┘
```

(The rail's `●` is the orchestrator's status and `•` its needs-you dot; `›` pops it open.)

### 4.4 The task view

A task opened is the task first, whole, and its work beneath. A task need not have a worktree or a
terminal at all (ov-79).

- **Header:** the key and title, and a status pop-up that replaces the card's "Move To" menu.
- **The task's text,** never collapsed: its blocked line and times, intent, acceptance (each line met or not, as
  the runner records it; the Mac has no write to tick one), constraints, the record of notes, and, in Needs
  Decision, the question with its **Answer** buttons. This replaces the modal card sheet.
- **Its work, beneath a draggable divider** whose place is remembered per window (the text starts at 40%):
  - a line naming the worktree and its terminal count ("Worktree fc-3-webhooks · 2 terminals"), the agent or a
    picker when several are on the task, and **Open Worktree**;
  - the tmux layout holding the agent's terminal, drawn as selecting it draws it;
  - the worktree's `ChangesPane`, under the agent, with a divider of its own (ruling 6: today's Changes view,
    reused without a tmux pane, so the agent's window keeps its size on every client; see R3). In Review the
    changes take the larger share; otherwise the agent does.
- **With no agent,** the line says "No agent is working on this task.", and the changes fill the work area when
  the task has a worktree. With neither an agent nor changes to show, the work area is that one line ("Nothing
  has started on this task yet."), and the text takes the rest. Never a full-height placeholder.

**Open Worktree** opens the worktree in the task's place, beside the board, with its own layouts (a `TileView`
and `GroupBar`, as a worktree opened from the sidebar draws). The breadcrumb reads Workspace › bil-9 … › fc-3-webhooks, and Back goes
along it.

### 4.5 The sidebar

Optional since ov-86, and hidden in a new window: the title bar's switcher and the board list do its work (§4.11).

```
◉ Needs You                         3     ← selects .needsYou
overnight                          ⋯ ＋   ← repository header (collapse and menus as today)
  Main                     ●         1   ← workspace row
  Billing                  ●         2
    ▾ Worktrees                             ← disclosure on the workspace row
        fc-3-webhooks · bil-9   +42 −7
        main checkout
        scratch-shell
  Relay rewrite            ◌
  Unclaimed  1                            ← collapsed, as today
  Hidden  2
```

**A workspace row** shows:
- the name;
- the orchestrator's `StatusGlyph`, or `◌` when there is no orchestrator;
- the needs-you count, in the amber of `AttentionBadge`.

The task prefix is in the tooltip and the window subtitle, not on the row. The row takes the header's context menu
from ov-60 (`WorkspaceActions.swift:13-32`): Show Board, Start Orchestrator or Replace Orchestrator…, Show
Charter. It stays a drop target for dragging a worktree onto it (`SidebarViews.swift:1369-1418`).

**Worktrees** lists the workspace's worktrees in the runner's order (`worktree.reorder`, unchanged).
- A row is: the worktree's name, then its task key(s) from §3.2's `tasks`, its +/− counts, and its attention dot.
- Expanding a worktree lists its terminals, as today.
- Selecting either drills into it (`focus: .worktree`): Workspace › Worktree.
- Worktrees opens by itself when the selection is inside it.
- Its rows are draggable to another workspace row, as today. They also gain a **Move to Workspace ▸** menu item,
  so the drag has a menu equivalent.

**Rows that go away:**
- the per-workspace **Board** and **Orchestrator** rows (`ContentView.swift:921-957`);
- the header's global attention badge (`:1280-1293`), since the Needs You row replaces it.

### 4.6 Needs You on the Mac

Selecting **Needs You** fills the detail with the merged item list, in one column no wider than about 720 pt.
- Each row shows the workspace name, the question, and the task key or agent, with its buttons trailing.
- Clicking the row body opens the item as §2.5 says.
- An item that's answered animates out.
- With nothing waiting: "Nothing needs you" / "Asks, decisions and reviews from every workspace show up here."

**Launch:** the app opens on Needs You when the count is above zero; otherwise, on the last workspace selection
(the same rule as the iPhone, ruling 4).
This replaces `selectFirstRunningTerminal` (`ContentView.swift:2100`).

### 4.7 tmux layouts

Nothing on the runner changes.
- A worktree is still a tmux session, and a layout is still a tmux window (`Layout.swift:3-15`).
- The Layout menu, `⌃B` verbs, split, zoom and drag-to-place keep working in whichever `TileView` has focus: the
  task's agent, an opened worktree, or the conversation.

What changes is where a layout is reached from:
- A worktree's layouts move one click down, under its task or Worktrees.
- The `GroupBar` shows only when a worktree is opened whole. A task's agent is shown as the one layout holding it.

The orchestrator's window stays out of the main checkout's layouts (`ownLayouts`, `WorkspaceSidebar.swift:366-370`).

### 4.8 The ⌘P palette

`PaletteAction` (`PaletteIndex.swift:10-21`) gains two cases:
- `.openWorkspace(host, id)`: rows such as "Billing" with the detail "Workspace · overnight". A match keeps its
  workspace, so a workspace with no matching worktree is no longer dropped (`WorkspaceSidebar.swift:217`).
- `.openTask(host, id)`: rows such as "bil-9 Invoice PDF export" with the detail "Billing · In Progress", matched on
  key and title, from the boards already loaded.

**Orchestrators** are listed as "Billing Orchestrator".

**Terminals** stay, and each lands in its workspace per §4.2. `.newTask(String)` is renamed `.newWorktree` in code,
and its title stays "New Worktree…".

The sidebar's search field (Edit ▸ Find, `Commands.swift:301`) becomes "Find Workspace, Task or Agent", and it
filters workspace rows and Worktrees.

### 4.9 The attention cycle and other commands

- **⌃⌘N, Next Needing Attention** (`Commands.swift:142-143`): walks the merged items in rank order from the
  current one, opening each as its Open action would. It used to walk terminals in sidebar order
  (`ContentView.swift:2656-2663`, `WorkspaceSidebar.swift:146-161`), which never reached a decision.
- **⌘] and ⌘[, ⌥⌘↓ and ⌥⌘↑, ⌃⌘1…** (⌘1… until ov-86)**:** step through the terminals of the view on screen. That's the task's layout, the
  opened worktree, or the conversation. They no longer step through the whole sidebar, since the sidebar no longer
  lists terminals.
- **⇧⌘B, Show Board:** selects the current workspace and focuses its board column. With nothing selected and one
  repository, it opens Main. The refusal copy "choose a project's Board" (`ContentView.swift:2693`) becomes
  "Select a workspace first."
- **Focus shortcuts:** ⌥⌘1 selects the orchestrator, ⌥⌘2 the navigator, ⌥⌘3 the main area, whatever it shows
  (ov-92; ov-89 popped the orchestrator open from its rail and the board from its strip). ⌘digits are the workspaces since ov-86 (§4.11); the terminals' moved to ⌃⌘digits.
- **Back** is ⌃⌘← and **Focus** is ⌃⌘↩. The usual chords are taken: ⌘[ is Previous Terminal
  (`Commands.swift:136-137`) and ⇧⌘↩ is Zoom Pane (`:170-171`). ⌃⌘←, ⌃⌘↩ and ⌥⌘1–3 are unused in `Commands.swift`
  and aren't in `ShortcutSheetTests.systemChords`. Back goes from a worktree to the task it was opened from, and
  from a task closes it, back to the board. Esc also goes back, but only when no terminal has focus, since a
  terminal needs its Esc.
- **Window title:** the workspace's name, with the subtitle "repository · runner". With a task focused, the title
  is "bil-9 Invoice PDF export" and the subtitle is "Billing · overnight". This replaces the worktree-derived
  title (`Model.swift:178-187`) everywhere except a loose worktree.

### 4.10 The orchestrator's conversation

The conversation column draws the orchestrator's terminal as selecting its row does today:
- a VT grid in terminal mode;
- `AgentSurface` in agent mode (`pane_mode`, `proto/farcooler.proto:874`).

The existing **Terminal ⟷ Chat** toggle (⌃B a, `Commands.swift:217`) and the palette's `togglePaneMode` work
there. The column doesn't choose a mode of its own; the pane's mode is the runner's, which every client shares.
New orchestrators start in chat when `agents.preferChatMode` is on (`Preferences.swift:78`), as other agents do.

The column header shows:
- `Orchestrator`, then the harness (`claude`) and the status glyph;
- a `⋯` menu with Replace Orchestrator…, Show Charter, Terminal / Chat, and Restart.

---

### 4.11 Getting around without the sidebar (ov-86)

The owner (2 Oct): a workspace switcher in the title bar, and a way between worktrees from inside one. The
sidebar becomes optional; everything it did is reachable without it.

- **The workspace switcher**, the toolbar's leading item, after the traffic lights: "Billing · shop ⌄", the
  workspace and its repository. It's a native `NSMenu` (`WorkspaceSwitcherButton`, review M3), so arrow keys,
  type-select, Return and VoiceOver work; ⌘0 (Workspace ▸ Switch Workspace…) opens it. Its lines are
  `WorkspaceSwitcherMenu.entries`: every workspace grouped by repository under section headers, in the
  sidebar's order (`WorkspaceNumbers.groups`), with the waiting count as the item's badge, an amber dot, and its
  ⌘-number as its key equivalent; Repositories ▸ with each repository header's Reconnect, New Terminal in
  Checkout and Remove Repository…; the runners' state with Reconnect for each in trouble; then Needs You, Go to
  Anything…, New Workspace…, New Worktree…, Add Repository…, Add Device or Runner… and Runners and Devices…
  (Settings ▸ Runners). The Needs You tray sits beside it. The window has no subtitle in a workspace: the
  switcher says the workspace and repository.
- **The runner banner** (`RunnerBanner`) across the top of the detail carries the sidebar status bar's trouble,
  sidebar or not: the runners' state while it's trouble, Reconnect for each runner in trouble, and
  `DaemonUpdateBar`.
- **Nothing is reachable only from the sidebar.** `SidebarAction` lists what it does; `SidebarParityTests`
  checks the switcher, the banner, the navigator (`Navigator.offers`) and `WorktreeMenu` offer every one. A worktree's menu (`WorktreeMenu.items`:
  Open, Show Changes, New Terminal, Move to Workspace ▸, Use as Orchestrator, Hide or Unhide, Remove Worktree…)
  is on its row under Worktrees, on a task row (Worktree ▸) and at the end of the breadcrumb's menu. A
  workspace row's Show Board, Start Orchestrator and Show Charter are the navigator's orchestrator row (ov-92;
  the rail until then) and the conversation header. Settings ▸ Runners has Reconnect All; Remove Repository… is only in the switcher there,
  since Settings has no fleet.
- **The board list is the navigator inside a workspace** (§4.3). Each task's row names its worktree after its
  key (`⎇ tax-rounding`; the task's own worktree, else its agent's, as §4.4 draws beneath it). The
  **Worktrees** section, apart from the tasks since ov-92, lists the worktrees no task on the board names
  (`WorkspaceWorktrees.loose`): those the workspace owns, and, for Main, its repository's unclaimed ones, which
  open beside Main's navigator as a loose worktree. Selecting one shows it whole in the main area. Its last row
  is **New Worktree…**; a row's menu has Hide (never the main checkout), and hidden ones collapse under
  **Hidden** with Unhide. Rows are drawn without seated orchestrators (`ownTerminals`), so the orchestrator's terminal is never
  listed there (§4.7's one place).
- **The order** (`WorkspaceWorktrees.entries`): the order the list draws (review M2): sections in `order`, each
  section's rows as `visibleRows` draws them (Done newest first), a worktree once under its first task; then
  the Worktrees section's, then its Hidden ones. Collapsed sections, Done's cut tasks and hidden worktrees are
  walked too, in the place they'd be drawn, so nothing is reachable only from the sidebar.
- **The breadcrumb's last segment is a menu**, "Billing › bil-3 Tax rounding › tax-rounding ▾" after the branch
  symbol (one glyph, `WorktreeSection.glyph`, on task rows, the section and the breadcrumb): the workspace's
  worktrees, task ones labelled with their task and the worktree beneath, then the loose ones, the current one
  checked, then that worktree's own menu. It stands for a worktree opened whole in place of its crumb, and
  follows a task as the worktree beneath it ("Worktrees" for a task with none; `WorkspaceWorktrees.crumb`). A
  task's worktree is gone to as its task.
- **Keys.** ⌃⌘↓ and ⌃⌘↑ (Workspace ▸ Next/Previous Worktree) walk the order, wrapping, from the task or
  worktree open, or from the board alone to the first or last. ⌘1–⌘9 select the first nine workspaces in the
  switcher's order. They were Terminal 1–9, which moved to ⌃⌘1–⌃⌘9 (free, and not a system chord;
  `ShortcutSheetTests.numbersAreWorkspaces`). ⌘0 opens the switcher. These, and ⌃⌘↑/↓, act only with the
  main window key and nothing over it (`MainWindowFocus.navigates`), and a command acts in the key window
  alone. ⌘P lists a worktree before the terminals found only by its name (they score
  `PaletteIndex.locatedPenalty`, 12, under it) and finds it by its open tasks' keys.
- **The sidebar's default** (`SidebarDefault`): `NavigationSplitView` takes a `columnVisibility` read from
  `window.sidebar`, written on every change. With nothing stored, it's hidden, unless the app has history here
  (`SidebarDefault.historyKeys`: a saved selection, the sidebar's open workspaces or collapsed repositories, a
  Settings tab) and AppKit's saved split view doesn't say it was collapsed: someone who used it before keeps
  the sidebar as they had it. ⌘B (View ▸ Toggle Sidebar) and the toolbar button still toggle it. With it hidden, ⌘F opens
  the palette, since the sidebar's search isn't there.

## 5. The responsive board

One `TaskBoardView`, in one form: the list.

**Superseded (ov-83).** This section first specified two forms chosen by the board's own width: the list below
892 pt and a kanban of 260 pt columns from there up, with 24 pt of hysteresis, and a `≡`/`▦` toggle in the board
header that forced either, kept per device as `board.form.<host>.<workspace>`. The owner removed the kanban, the
toggle and the thresholds: the board almost always sits in a narrow column beside the conversation, where the
kanban showed fewer statuses than the list does. `BoardForm` now holds only the list's rules.

**The header** is one row on the board column's grid (ov-83): the workspace's title at column B, the waiting
count beside it, and New Task… (**+**) and Refresh as icons at the trailing edge.

**The list form:**
- **Sections:** every status in `TaskBoardModel.order` (`AK/TaskBoardModel.swift:406-407`), always.
- **An empty status:** a collapsed header reading "Backlog 0", which can't be expanded.
- **A status with tasks:** expanded by default. Done and Canceled start collapsed, and their collapse state is
  remembered per workspace.
- **Card rows:** the iPhone's row (key, title, call to action, time, acceptance, agent control), ported from
  `ios/TaskBoardView.swift` to the shared view layer where it can be shared.
- **Selecting a row** opens the task beside the board (§4.3, §4.4); selecting it again closes it, and ↑/↓ step
  through the rows shown.

**Every form, on every platform:**
- `TaskBoardModel.sections` keeps every status with its count. It's added beside `listed`, which drops empty
  statuses (`AK/RunnerBoards.swift:246-257`; Kotlin `android/model/TaskBoard.kt:229`). `listed` stays, deprecated,
  until both phones have moved off it (`ios/TaskBoardView.swift:166`, `android/ui/BoardScreen.kt:283`), and is
  deleted in the last phone task. The phones take `sections` too: owner decision 3 applies there as well.
- **An implicit board always gets a row,** like a workspace's. An implicit board is a repository's board on a
  runner without `workstreams`. Today it's hidden when empty (`AK/RunnerBoards.swift:170`, Kotlin `TaskBoard.kt:508`),
  but §4.2 and §8 treat it as a workspace, so an empty one has to be reachable. A workspace's empty board already
  gets a row (ov-56).
- **New Task…** (`＋` in the header) files a task on this board with `task.create`, which is Control scope and
  already exists. The empty state "Put a task on it with farcooler task create." (`TaskBoard.swift:396-397`)
  becomes a button. In scope (ruling 5), as is Move to Workspace ▸ (§4.5).

---

## 6. The phones

The same structure on iOS and Android, each in its platform's idiom.

```
Needs You (front door)
├── [items, answerable in place]
├── Workspaces
│   └── overnight  (runner, when more than one)
│       ├── Main            ●  1
│       └── Billing         ●  2   ─►  Billing
│                                      [Orchestrator | Board | Worktrees]
│                                      ├── Orchestrator: the orchestrator's pane (chat or terminal)
│                                      ├── Board: the sectioned list  ─► Task
│                                      │                                 ├── the card (answer here)
│                                      │                                 ├── Agent  ─► its pane
│                                      │                                 └── Changes / Worktree
│                                      └── Worktrees: worktrees  ─► the worktree's panes
└── Settings, Runners
```

### 6.1 iOS

**Today:**
- The app opens onto a full-screen pane (`ios/FarCooler/FarCoolerApp.swift:80-89`).
- Workspaces are reached through a gesture-only overview.
- The board is a sheet (`ios/ShellScreen.swift:1150`).

**The new root is a `NavigationStack` whose root view is Needs You:**
- **Needs You:** the merged items, then a "Workspaces" section per runner and repository. Each workspace row shows
  its name, its orchestrator's glyph, and its count. Pull to refresh.
- **The workspace screen:** the workspace's name as the title, and a segmented `Picker` under the navigation bar.
  - **Orchestrator:** the orchestrator's pane, full height, using the existing terminal and agent views. With no
    orchestrator, see §8.
  - **Board:** the list form of §5, in-line, not a sheet. `BoardSheetHost` retires.
  - **Worktrees:** the workspace's worktrees, with their task keys and counts. **New Worktree…** at the bottom claims
    the new worktree for this workspace (ruling 8). Today the client core always claims for Main
    (`crates/client/src/session.rs:1005-1025`); it gains a `workspace` argument.
  - The segment is remembered per workspace.
- **The task screen:** the card, with Answer buttons in Needs Decision, then rows for Agent, Changes and Worktree.
  Each row pushes. Back returns to the task. This fixes the jump that closed the board.
- **The worktree screen:** today's pane pager and bottom bar, scoped to one worktree.
  - Swiping sideways on the bar moves between that worktree's terminals, not across the fleet.
  - The drag-up overview retires; the stack replaces it.
  - The "Diff" tab is renamed "Changes".

**Launch** (ruling 4): Needs You when it has items. Otherwise the last workspace, pushed on top of the Needs You
root so Back still reaches it.

**Deep links** (`farcooler://terminal/<id>`, from notifications and widgets) push the workspace, the task (when the
pane has one), and then the agent, so Back walks up that chain.

### 6.2 Android

**Today:**
- The front door is Needs You, grouped by worktree (`android/ui/Navigation.kt:57-59`; `android/model/NeedsYou.kt:182`).
- The worktree list is a drawer and a pushed route.
- The board is a pushed route.

**Changes:**
- **Needs You:** its model becomes a rendering of the rollup. Sections by worktree go away; each item is a row
  labeled with its workspace. The Board-row band (`NeedsYouScreen.kt:276-280`) goes, since decisions are now items.
  The Workspaces list follows, as on iOS.
- **Routes:**
  - `Board` becomes `Workspace(host, workspace, tab)`, with `tab` one of `orchestrator | board | worktrees`.
  - `BoardTask` stays; it's the task screen.
  - `Terminal` stays; it's the worktree screen.
  - `Fleet` retires. The drawer holds the Workspaces list.
- **The workspace screen:** a `TabRow` of Orchestrator, Board and Worktrees under the top app bar.
- **The worktree screen:** gains a back arrow when it's reached from a workspace. Today it has a hamburger
  (`android/ui/TerminalPane.kt:447-449`).

### 6.3 What replaces the board sheet

On both phones, the board is a tab of the workspace screen. A task is a pushed screen in the workspace's stack.
Going from a task to its agent pushes, too. So the board is never covered or dismissed by a jump.

---

## 7. Glance surfaces: watch, widget, Live Activity, push

Each of these reads the same items.

- **Push.**
  - **The contract** (camelCase, as the body's other renamed fields are: `startedAt`, `traceAnchor`,
    `crates/daemon/src/push.rs:212, 276`):
    - every agent notice gains `workspace` (the name, or absent) and `needsYou` (this runner's item count at the
      moment of sending);
    - a **decision notice** has `kind: "decision"`, `task` (the key), `workspace`, `title`, `subtitle` and
      `needsYou`, and **no `terminal`**. The relay alerts on it and writes no roster row, since rows are one per
      `(account, terminal)` (`services/relay/migrations/0008_fleet_rows.sql`);
    - a **count notice** has `kind: "count"` and `needsYou` only. The daemon sends one after any
      `needs_you_changed` that no other notice carried: at most one per 5 s window, opened by the first change and
      read at its close, so the last count of a burst is the one sent. A count the relay already has, from any
      notice that landed, is not sent again. The relay refreshes the card on every count notice at APNs priority
      10, so the daemon does the pacing. So answering a decision or a chat ask, which changes no
      terminal, still updates the lock screen. It alerts nothing.
    - A notice with no `kind` is an agent notice, as today.
  - The title leads with the workspace, as ov-60 did on the Mac: "Billing · claude needs you". For an
    orchestrator, it's "Billing Orchestrator needs you".
  - **A task entering Needs Decision sends a push** (ruling 3): "Billing · bil-7 needs a decision", with the
    question as the subtitle. Today nothing pushes a decision (workstreams spec, "Wake-ups and events").
- **Relay.** Today the header counts blocked rows (`services/relay/src/push.ts:269-285`), which leaves out
  decisions.
  - **Storage:** migration `0010_needs_you.sql` adds nullable `needs_you INTEGER` and `needs_you_at INTEGER` to
    `daemons`, the row for each daemon token. Additive and nullable, as 0002 through 0009 are.
  - **Update:** every notice that carries `needsYou` overwrites that daemon's two columns. It never adds.
  - **Sum, per runner:** each of the account's unrevoked daemons contributes its own `needsYou` when it has one
    newer than `ROW_RETENTION_MS` (24 hours, `index.ts`), and its blocked rows when it doesn't. The header's
    `needsYou` is the total. Runners upgrade one at a time, so one runner's count must never stand in for
    another's blocked agents. To tell whose a row is, migration `0012_row_daemon.sql` adds `daemon_id` to
    `live_activities`; a row from before it is matched by its runner label. With no daemon counting at all,
    `needsYou` is absent and the header falls back to today's `blocked` count, unchanged.
  - **Staleness:** a count clears in four ways. The next notice from that daemon overwrites it. A daemon silent
    for 24 hours drops out of the sum, and `readFleet`'s lazy purge nulls its columns. A revoked daemon is never
    summed. Pairing a new token under a label the account already has clears that label's older counts, so a
    re-paired runner isn't counted twice.
- **Live Activity.**
  - The header count is the rollup's count.
  - The rows stay the relay's agent rows, which are per terminal, not items. They're relabeled "Billing · claude"
    from the notice's `workspace`.
  - Asks get Allow and Deny buttons (ov-54).
- **Widget.** `FleetSnapshot` (`AK/FleetSnapshot.swift:18-30`) gains optional `needsYou: [Item]`, written by the app.
  The widget's count and rows come from it. An older snapshot, without the field, decodes as today.
- **Watch.** The watch has no sockets. It gets items the way it gets agents today: the iPhone's
  `WatchLinkHost.send(snapshot:)` (`ios/WatchLinkHost.swift:215-255`) puts `FleetSnapshot`, now with `needsYou`, in
  the application context.
  - The list's first section is Needs You.
  - Asks answer in place, as today.
  - Decisions and reviews show the question and "Open on iPhone".
  - The complication shows the count.

---

## 8. Empty and first-run states

| State | Mac | Phones |
|---|---|---|
| A repository with only Main, no orchestrator, and an empty board | Sidebar: "Main ◌". The workspace view shows the orchestrator's empty state in the main area and the board on the right, with every status at 0 and **New Task…** | The same, on the Orchestrator tab. Main's row is always listed; it isn't hidden for being empty (today's phones hide empty boards, `AK/RunnerBoards.swift:131-136`) |
| A workspace with no orchestrator | The conversation column is centered: "No orchestrator" / "An orchestrator runs this workspace's board. It reads the charter, dispatches agents, and asks you when it needs a decision." **Start Orchestrator** is a pop-up offering Claude, Codex and Cursor (`OrchestratorHarness`, from ov-60). While it starts: "Starting Orchestrator…". A refusal becomes a banner, in ov-60's words | **Start Orchestrator** with a harness menu, sending `workspace.start_orchestrator` (Control). Phones haven't sent it before; ruling 8 supersedes the workstreams spec's "no workspace management" line for this action |
| An orchestrator whose pane was lost | The column shows the lost pane's last screen dimmed, with **Restart** (`terminal.restart`, which resumes the conversation) and **Replace…** | The same actions, in a menu |
| An orchestrator that is starting and was never confirmed | "Starting Orchestrator…", then after 30 s: "This is taking longer than usual." with **Replace…**, since the seat can stick (CLI map §7.11) | The same |
| A workspace just created, from a split | It appears in the sidebar with `◌`. The new orchestrator starts on the manager skill and reads its handoff by itself (ov-59: `orchestrator.rs:166-173`) | The same |
| A workspace with no worktrees | "No worktrees yet. The orchestrator makes them as it dispatches tasks." **New Worktree…** | The same |
| Nothing needs you | §4.6 | "Nothing needs you", with a caveat when a runner isn't answering (Android's existing line, `reassurance`, `android/model/FleetReading.kt:176`) |
| No runners | Unchanged onboarding | Unchanged onboarding |

---

## 9. Migration

Nothing on a runner migrates. The daemon adds an RPC, an event, a capability and one `Worktree` field; tmux
sessions, windows, worktree order and claims are untouched.

**Mac** (`UserDefaults`):
- `fleet.lastTerminal` (`ContentView.swift:89`) is read once and mapped to a `Selection` by §4.2's table. The
  result is written to `workspace.lastSelection` as `host | workspace | task-or-worktree`, and the old key is
  removed.
- `sidebar.collapsedProjects` (`Preferences.swift:46`) keeps its keys and its meaning, which is repository
  collapse.
- Expanded worktree rows were `@State` (`ContentView.swift:11`) and never persisted, so nothing is lost.
  - Worktrees disclosure state is new: `sidebar.openWorktrees`, empty by default.
- New: `board.form.*` (§5), and the task view's dividers.
- **The first launch after the update** shows a one-time tip over the sidebar:
  - "Workspaces are now in the sidebar."
  - "Select one to see its orchestrator and board side by side. Its worktrees are one click down."
  - The button is **OK**.
  - Superseded (ov-85): the tip now teaches the board-and-task layout, under a fresh key
    (`tips.tasksBesideBoard`), so it shows once even to whoever dismissed the first: "Tasks now open beside the
    board." / "Click one to open it. Press ↑ or ↓ to look through the others, and Esc to close it."
  - Superseded again (ov-89), under `tips.orchestratorFillsWorkspace`: "The orchestrator now fills the
    workspace." / "Your board is on the right. Click a task to open it here, press ↑ or ↓ to look through the
    others, and Esc to go back to the orchestrator."
  - Superseded again (ov-92), under `tips.workspaceNavigator`: "Your workspace has a navigator." / "The
    orchestrator, your tasks and your worktrees are listed on the left. Pick one to show it here, use ↑ and ↓
    to move through them, and press Esc to go back to the orchestrator."

**iOS:**
- Each worktree's remembered terminal (`ShellFleetMap.resume`, `ios/ShellScreen.swift:1943`) carries over to the
  worktree screen.
- The crossing note (`:1823`) retires.
- The Quick Task drafts (`quicktask.*`, `ios/TaskComposer.swift:28-31`) move to the New Worktree sheet's keys.

**Android:**
- The saved back stack (`android/ui/Navigation.kt:315-328`) is decoded with the old `@SerialName`s kept as aliases:
  - `Board` becomes `Workspace(tab = board)`;
  - `Fleet` is dropped;
  - a stack that doesn't decode falls back to the root, as it already does.

**Watch and widget:** new fields are optional, so old snapshots decode.

---

## 10. Slices

Each slice ships on its own and is usable without the next.

### Slice 1: the rollup and the links (data)

**Contents:**
- **Daemon:**
  - `needs_you.list`, `needs_you_changed` and the `needs_you` capability;
  - item assembly from `hook_asks`, `AgentSupervisor`, activity and tasks;
  - `Worktree.tasks`;
  - push `workspace` and `needs_you`, and the relay's per-machine sum.
- **`crates/client`:** `needs_you_json`, `Session::needs_you`, and the FFI entry.
- **CLI:** `farcooler needs-you [--json]`.
- **AgentKit and Kotlin:** the decode, the rank merge and `TaskLink.task(of:in:)`, against one fixture.

**Usable alone:**
- The CLI command, for a person or an orchestrator.
- The lock screen's count includes decisions, through the relay.
- Android's panes name their task. The Mac and iOS consumers moved into slices 2 and 4, which own those apps'
  files.

**Tests:** see §11 R6 for the ones that must go red first.

### Slice 2: the Mac workspace view

**Contents:**
- the new `Selection` and its mapping;
- the sidebar of §4.5;
- the three columns and their collapse rules;
- the task column;
- Needs You;
- the palette, the commands and the migration.

The board column hosts today's `TaskBoardView` until slice 3 lands, in list form if slice 3 has shipped and
otherwise as the kanban. That kanban is cramped in 340 pt but works, since it scrolls sideways.

**Depends on:** slice 1, for Needs You and the counts.

### Slice 3: the responsive board

**Contents:**
- `sections` replaces `listed` in AgentKit and Kotlin;
- the list form on the Mac;
- the width rule and the toggle;
- the task column replaces the card sheet;
- New Task….

**Usable alone:** yes. On today's full-width board, a narrow window gets the list instead of one visible column.

### Slice 4: the phones

- **4a, iOS:** the root stack, the workspace screen, the task screen, the scoped worktree screen, Start
  Orchestrator.
- **4b, Android:** the same.
- **4c, glances:** the watch, the widget and the Live Activity reading the items.

**Depends on:** slice 1, and on slice 3's `sections`.

### What can run in parallel

The plan (`docs/superpowers/plans/2026-09-28-workspace-ui.md`) assigns every shared file to one owning task and
fixes the groups:

1. **Day one:** the daemon (1A), the relay (1B), the shared board rules then the Mac board (3A → 3B), and the
   column measurement (2A).
2. **After 1A:** the client core and the CLI (1C).
3. **After 1C:** AgentKit (1D) and Kotlin (1E).
4. **After 1D, 1E, 3A, 3B and 2A:** the Mac (slice 2, one lane), iOS (4A), Android (4B), and the glances (4C, after
   4A.1).
5. **After 4A and 4B:** deleting `listed` (4D).

---

## 11. Risks

- **R1. The iOS navigation rewrite is the largest piece.** The app was built "onto terminals" on purpose
  (`FarCoolerApp.swift:80-89`), and its gesture shell is tuned (the `ios-shell-mechanics` brief).
  - Mitigation: slice 4a keeps the shell intact as the worktree screen and changes only what's above it.
  - UI tests go through `scripts/ios-ui-tests.sh`.
- **R2. Terminal width in a column.** Resizing a tmux pane resizes it for every client: the Mac's `onGeometry`
  resizes the terminal (`ContentView.swift:1859`). An orchestrator drawn in a 412 pt column reflows to 48 columns
  on the phone as well.
  - Nothing arbitrates this. `size_controller_client_id` is never set (`crates/daemon/src/wire.rs:302` always
    sends `None`), and tmux runs `window-size latest` (`crates/tmux/src/server.rs:53`). The last client to
    resize a pane sets its size for everyone.
  - Measured in 2A: a phone showing the pane notices the Mac's resize within its 2-second geometry poll
    (`TerminalSession.checkGeometry`, `geometryInterval`). It reopens at the Mac's width. It doesn't resize the
    pane back until its own viewport changes or the app returns to the foreground (`reassertSize`). When it
    leaves the pane, it hands back the shape it found (`releasePane`). The Mac reports only when its geometry changes
    (`lastReportedGeometry`), so the two don't fight.
  - For the conversation this costs nothing. An iPhone is 47–48 columns (SF Mono 13 on 393–402 pt), which is
    the conversation's minimum.
  - For the task column, at 58 columns or more, a phone opening the same agent narrows it to 48 until the Mac
    lays out again.
  - The 412 and 489 pt minimums were measured in 2A (§4.3) from the real captures of claude, codex and cursor.
    No harness was run live: Far Cooler's lanes don't run the real agents. Claude 2.1.283's newest permission
    lines (76 and 82 columns) are wider than any minimum here, and wrap once.
- **R3. The task column's Changes lives outside tmux.** `ChangesPane` sits in a tmux pane so that zoom, drag and ⌃B
  reach it (`ChangesPane.swift:19-27`). In the task column the same view is drawn without the pane, so there it
  can't be zoomed or dragged. The Changes toolbar button in an opened worktree still makes the tmux pane.
- **R4. The dispatch link isn't atomic.** Dispatch opens the pane with `task_id`, then sets the task's
  `worktree_id` in a second call (`crates/cli/src/tasks.rs:2588-2597`). If the second call fails, the pane knows its
  task but the task doesn't know its worktree.
  - §3.2's fallback covers the display.
  - Moving the lane update into the daemon's dispatch is a follow-up.
- **R5. A finished agent with no task is in no inbox.** Ruling 1 keeps Done agents out of the rollup, so work an
  agent finishes outside a task shows only as its `✓` and its notification. Android's front door lists them today,
  so its users lose that list.
- **R6. Checks that can't fail.** The rollup's tests must each be broken once on purpose and seen to go red:
  - one item per subject;
  - the kind precedence;
  - the rank merge across two runners with skewed clocks;
  - the fixture that AgentKit and Kotlin both decode;
  - the count agreeing between the RPC and the push.
- **R7. Older runners and newer apps.** §2.6 degrades a runner without `needs_you` honestly, but the Needs You
  count can differ between a Mac that talks to an old runner and a phone that talks to a new one only if the
  runners differ. Every app updates together, so this is transient.

---

## 12. Rulings

The owner delegated these to the coordinator, who ruled on 2026-09-28. Each is decided.

1. **"Ready for review" is tasks in In Review only.** No unread diffs and no finished agents. *Why:* review is a board
   state the orchestrator sets on purpose; diffs and finished turns would flood the inbox.
2. **The inbox opens a review; it doesn't approve one.** *Why:* the charter says who lands work, often the
   orchestrator, and one tap in a list shouldn't bypass it.
3. **Decisions send a push.** *Why:* decisions are now in the count, and a lock-screen count that rises silently is a
   count nobody trusts.
4. **The iPhone opens to Needs You when it has items, else to the last workspace.** The Mac follows the same rule.
   *Why:* the front door is the inbox, but an empty inbox is a screen with nothing to do.
5. **New Task and Move to Workspace on the Mac are in.** *Why:* both are small, and an empty board and an
   undiscoverable drag were dead ends.
6. **The task column reuses today's Changes view as it is.** Today it's a native SwiftUI view drawn in a tmux pane's
   rectangle (`TileView.swift:416`); the column embeds the view directly, without the pane. *Why:* no new renderer,
   and nothing resizes the agent's window for other clients.
7. **The list/kanban choice is kept per device, not synced.** *Why:* the phone is always a list, so there's nothing
   to sync. (Moot since ov-83, which removed the kanban: every board is a list.)
8. **Phones may start orchestrators, and a worktree created from a workspace's screen is claimed for it.** *Why:* a
   workspace with no orchestrator was a dead end on the phone, and new worktrees landing in Main was a surprise.
9. **"Helpers" becomes "Worktrees".** *Why:* the vocabulary stays, and "worktree" is already the word.
10. **An orchestrator's finished turn is an unread dot, not an inbox item.** *Why:* it finishes every turn, so it
    would always be in the inbox.
11. **iPad is out of scope; it uses the iPhone layout.** *Why:* nobody asked for it, and the iPhone structure works
    there.

Also ruled: slice 2 measures the three columns' minimums at a full-screen 13" width before fixing them (§4.3).
