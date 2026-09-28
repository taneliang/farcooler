# The workspace UI: implementation plan

Date: 2026-09-28
Spec: [`docs/superpowers/specs/2026-09-28-workspace-ui-design.md`](../specs/2026-09-28-workspace-ui-design.md) (decided; its rulings are in §12)
Status: not started

## How to read this

- **Paths:**
  - Mac paths are relative to `apps/macos/Sources/FarCooler/`.
  - `AK/` is `apps/shared/AgentKit/Sources/AgentKit/`, and `AKT/` is `apps/shared/AgentKit/Tests/AgentKitTests/`.
  - `MT/` is `apps/macos/Tests/CeremonyTests/`.
  - `android/` is `apps/android/app/src/main/java/com/farcooler/`, and `androidT/` is its `src/test` twin.
- **Test style per platform:**
  - Rust tests are named as sentences.
  - Swift uses Swift Testing: `@Test("…")`.
  - Kotlin uses backtick names.
  - The relay uses `it('…')`.
- **Every test named below is broken once on purpose and seen to go red before its task is done** (the repo's
  "checks that cannot fail" rule). The task's commit message says how it was broken.
- **The Rust tree is hand-formatted.** Never run `cargo fmt`. Cargo is off PATH; see the toolchain notes.
- **Lanes:** one lane per worktree. Tasks marked ∥ can run in parallel lanes. Tasks marked → wait for the tasks
  they name.
- **Merging:** no merge commits. Each lane is rebased or squashed onto `main`.

## The slices at a glance

| Slice | Starts | Depends on | Lanes |
|---|---|---|---|
| 1: the rollup and the links (data) | Day one | — | 1A daemon ∥ 1B relay; then 1C client core and CLI; then 1D AgentKit ∥ 1E Kotlin |
| 3: the responsive board | Day one, ∥ slice 1 | — | 3A shared rules (AgentKit ∥ Kotlin); then 3B Mac |
| 2: the Mac workspace view | After 1D; after 3B for the board column | 1D, 3B | 2A measurement first; then 2B–2F |
| 4: the phones | After 1C–1E and 3A | 1C, 1D, 1E, 3A | 4A iOS ∥ 4B Android ∥ 4C glances |

Slices 1 and 3 share no files, so they start together.
- Slice 1 touches the daemon, proto, client core, CLI, relay, and the AgentKit and Kotlin *decode* files.
- Slice 3 touches `TaskBoardModel`, `RunnerBoards`, `TaskBoard.swift` and the Kotlin board model.

The one shared file is `proto/farcooler.proto`, and slice 3 doesn't edit it.

---

## Slice 1: the rollup and the links

### 1A. Daemon (one lane, in order)

**1A.1 Proto and capability.**
- `proto/farcooler.proto`:
  - `NeedsYouKind`, `NeedsYouItem`, `NeedsYouAction`, `TaskRef`, `TerminalRef`, `WorktreeRef` and `NeedsYouList`,
    per spec §2.4, with fresh tags;
  - `Event.needs_you_changed` (Empty), with a fresh tag;
  - `Worktree.tasks` (`repeated TaskRef`), with a fresh tag;
  - a `result` variant for `NeedsYouList`.
- `crates/protocol/src/lib.rs`:
  - `capability::NEEDS_YOU = "needs_you"`;
  - `"needs_you.list" => NEEDS_YOU` in `for_method`, beside `:459-476`;
  - the method name in the list at `:795` onward.
- Tests (`crates/protocol/src/lib.rs`):
  - `needs_you_list_is_gated_on_its_own_capability`
  - `every_new_needs_you_message_takes_a_fresh_tag`: extends the frozen-tag descriptor test.

**1A.2 Item assembly, as a pure function.**
- New `crates/daemon/src/needs_you.rs`:
  `fn assemble(inputs: &Inputs) -> Vec<pb::NeedsYouItem>`, where `Inputs` holds:
  - terminals, with activity, rank, `task_id`, `workspace_id`, role and `blocked_question`;
  - held asks, from `HookAsks`;
  - chat permissions, from `AgentSupervisor`;
  - tasks in Needs Decision or In Review, each with its latest `QUESTION` note;
  - worktree owners.

  No I/O, so every rule is unit-testable.
- Rules, per spec §2.2:
  - four kinds;
  - one item per subject, the most urgent winning, the rest in `also`;
  - rank tiers ask, blocked, decision, review, oldest first;
  - review is In Review tasks only;
  - no item for a Done agent;
  - no item for an orchestrator's finished turn;
  - the workspace fallback chain;
  - hidden worktrees count.
- Tests, in `needs_you.rs` `mod tests`:
  - `a_task_with_a_held_ask_and_a_decision_is_one_ask_item`
  - `an_ask_outranks_a_block_outranks_a_decision_outranks_a_review`
  - `within_a_kind_the_oldest_comes_first`
  - `a_done_agent_is_not_an_item`
  - `an_orchestrators_finished_turn_is_not_an_item`
  - `an_orchestrators_ask_is_an_item_about_its_own_terminal`
  - `a_blocked_codex_with_no_answerable_ask_is_a_blocked_item`
  - `a_failed_turn_is_a_blocked_item_until_seen`
  - `a_task_in_review_is_a_review_item_with_only_an_open_action`
  - `an_item_takes_the_tasks_workspace_then_the_terminals_then_the_worktree_owners`
  - `an_item_in_a_hidden_worktree_still_counts`
  - `an_unclaimed_worktrees_item_has_no_workspace`
  - `a_decisions_options_become_its_actions_and_more_than_three_go_to_a_menu`
  - `item_ids_are_stable_across_two_assemblies`

**1A.3 The RPC.**
- `crates/daemon/src/rpc.rs`:
  - `"needs_you.list" => Scope::Read`, beside `changes.inbox` (`:414`);
  - the dispatch arm, beside `:1855`, gathering `Inputs` from the store, `HookAsks` and the supervisor.
- Test: new `crates/daemon/tests/needs_you_reads_every_kind_over_the_socket.rs`, using `rpc_over_socket.rs`'s
  harness. It makes one of each kind in a scratch store and reads them back in rank order.

**1A.4 The event.**
- Announce `needs_you_changed` from:
  - an activity change in the watcher;
  - `HookAsks::hold` and `settle`;
  - the supervisor's `Permission` and `Resolved`;
  - `task_changed` into or out of Needs Decision or In Review;
  - `terminal.seen`.
- Coalesce the announcements the way `announce_fleet_changed` does (`rpc.rs:1144`).
- Tests, in `needs_you.rs` `mod tests`:
  - `settling_a_held_ask_announces_needs_you_changed`
  - `moving_a_task_into_needs_decision_announces_needs_you_changed`
  - `a_working_to_working_tick_announces_nothing`

**1A.5 `Worktree.tasks`.**
- Fill the new field in `worktree.list`'s converter: tasks whose `worktree_id` is the worktree and whose status is
  not Done or Cancelled.
- Emit `worktree_changed` for the old and new lane on a `task.update` that moves `worktree_id`.
- Tests:
  - `a_worktree_lists_its_open_tasks_and_not_its_done_ones`
  - `moving_a_tasks_lane_changes_both_worktrees`

**1A.6 Push.**
- `crates/daemon/src/watch.rs` and `push.rs`:
  - The notification carries `workspace` (the name) and `needs_you` (this runner's item count).
  - The title leads with the workspace: "Billing · claude needs you", or "Billing Orchestrator needs you" for an
    orchestrator.
- **A task entering Needs Decision sends a push** (ruling 3): "Billing · bil-7 needs a decision", with the question
  as the subtitle, on the done channel.
- Tests, in `watch.rs` `mod tests`:
  - `a_blocked_agents_notice_leads_with_its_workspace`
  - `an_orchestrators_notice_is_its_workspace_alone`
  - `a_task_entering_needs_decision_sends_one_push`
  - `every_push_carries_this_runners_needs_you_count`

### 1B. Relay ∥ 1A (one lane)

**1B.1 The per-machine count.**
- `services/relay/src/push.ts`:
  - Keep the latest `needs_you` per machine.
  - The content-state gains `needsYou`, the sum. Absent from an old daemon, it falls back to today's `blocked`.
- Tests (`services/relay/test/relay.test.ts`):
  - `it('sums the latest needs-you count per machine')`
  - `it('falls back to the blocked count when no machine sent needs-you')`
  - `it('replaces a machine\'s count rather than adding to it')`

### 1C. Client core and CLI → 1A.1 (one lane)

**1C.1 The JSON projection.**
- New `crates/client/src/needs_you_json.rs`: `pub fn needs_you_json(list: &pb::NeedsYouList) -> Value`, one shape.
- `Session::needs_you` in `session.rs`, beside `changes_inbox` (`:1639`).
- `"needs_you"` in the FFI call table (`ffi.rs`, beside `"fleet"` at `:1549`).
- `worktree_json` gains `tasks`.
- Tests (`needs_you_json.rs` `mod tests`):
  - `every_field_of_an_item_is_in_the_json`
  - `an_absent_task_is_null_not_missing`
- Write `test/fixtures/needs-you.json` from this function's output. It is the one fixture every client decodes.
  Test: `the_shared_fixture_is_what_needs_you_json_writes`, which fails when the fixture drifts.

**1C.2 The CLI.**
- `crates/cli/src/main.rs`: `farcooler needs-you [--json]`.
  - Plain output: one line per item, `<kind>  <workspace>  <key or agent>  <question>`.
  - Empty: `nothing needs you`.
- Tests (`crates/cli/src/main.rs` `mod tests`):
  - `needs_you_prints_one_line_per_item_in_rank_order`
  - `needs_you_json_is_the_client_cores_shape`
  - `needs_you_on_an_old_runner_says_to_update_it`

**1C.3 A workspace claim for phone-made worktrees** (ruling 8).
- `crates/client/src/session.rs:1005-1025`: `worktree_create` takes an optional `workspace` and claims for it,
  falling back to Main as today.
- Tests:
  - `a_worktree_made_with_a_workspace_is_claimed_for_it`
  - `a_worktree_made_without_one_is_still_claimed_for_main`

### 1D. AgentKit → 1C.1 ∥ 1E

**1D.1 Decode and merge.**
- New `AK/NeedsYou.swift`:
  - `NeedsYouItem` (Codable, tolerant of unknown kinds);
  - `NeedsYou.merge([runner: [NeedsYouItem]]) -> [NeedsYouItem]`, by rank, then runner name;
  - `count(in workspace:)`.
- Tests (`AKT/NeedsYouTests.swift`):
  - `@Test("The shared fixture decodes every field")`, reading `test/fixtures/needs-you.json` via `#filePath`, as
    `TaskBoardModelTests.swift:289` does.
  - `@Test("Two runners merge by rank, not by clock")`
  - `@Test("An unknown kind decodes and sorts last")`
  - `@Test("A workspace's count is its items, not its signals")`

**1D.2 The pane's task.**
- New `AK/TaskLink.swift`: `TaskLink.task(of: terminal, in: worktree)`. It returns `task_id` first, then the
  worktree's only open task, else nil.
- Tests (`AKT/TaskLinkTests.swift`):
  - `@Test("A dispatched pane's task is its own")`
  - `@Test("A pane in a worktree with one open task shows that task")`
  - `@Test("A pane in a worktree with two open tasks shows none")`
  - `@Test("A shell's inferred task never counts as the task's agent")`: `TaskAgentLink.isWorking` is unchanged.

**1D.3 Mac: the first consumers.**
- `DaemonClient.swift`: read `farcooler needs-you --json` on `needs_you_changed`; `FleetStore` merges runners.
- `ContentView.swift`:
  - `.nextAttention` (`:2656-2663`) walks merged items;
  - the header badge (`:1280-1293`) shows the merged count.
- Task chip in `GroupBar.swift`.
- Test (`MT/NeedsYouCycleTests.swift`): `@Test("⌃⌘N steps through items by rank and reaches a decision")`.

### 1E. Kotlin → 1C.1 ∥ 1D

**1E.1 Decode, merge and the task link.**
- New `android/model/NeedsYouItems.kt` and `android/model/TaskLink.kt`, mirroring 1D.1 and 1D.2.
- Tests:
  - `androidT/model/NeedsYouItemsTest.kt`:
    - `` `the shared fixture decodes every field` ``, reading `test/fixtures/needs-you.json` the way
      `GeneratedFilesTest.kt:218` finds repo files
    - `` `two runners merge by rank not by clock` ``
    - `` `an unknown kind decodes and sorts last` ``
  - `androidT/model/TaskLinkTest.kt`:
    - `` `a pane in a worktree with one open task shows that task` ``
    - `` `two open tasks show none` ``

**1E.2 Task chip.** In the worktree screen's top bar (`android/ui/TerminalPane.kt:408-452`).
Test: `` `the top bar names the pane's task` ``, in `androidT/ui/`.

**Slice 1 is done when:**
- `farcooler needs-you` lists one of each kind on a scratch daemon, which is killed by PID afterwards;
- the Mac's ⌃⌘N reaches a decision;
- a phone's pane names its task.

---

## Slice 3: the responsive board (∥ slice 1 from day one)

### 3A. Shared rules (AgentKit ∥ Kotlin)

**3A.1 `sections` replaces `listed`.**
- `AK/RunnerBoards.swift:246-257`: `TaskBoardModel.sections`, every status in `order` with its count, empty ones
  included. Delete `listed`.
- `AK/RunnerBoards.swift:125-172`: a workspace gets a Board row even when its board is empty.
- Tests (`AKT/RunnerBoardsTests.swift`):
  - `@Test("Every status is a section, empty ones included")`
  - `@Test("Needs Decision leads")`
  - `@Test("An empty board still has a row")`

**3A.2 The form rule.**
- New `AK/BoardForm.swift`:
  - `BoardForm.resolve(width:, previous:, forced:) -> .list | .kanban`, switching at 824 going up and 800 going down;
  - `BoardForm.Choice` (`auto | list | kanban`), stored per device under `board.form.<host>.<workspace>`.
- Tests (`AKT/BoardFormTests.swift`):
  - `@Test("Below 800 it's a list, at 824 and up a kanban")`
  - `@Test("Between 800 and 824 it keeps the form it had")`
  - `@Test("A forced form ignores width")`
  - `@Test("Choosing the forced form again returns to Automatic")`

**3A.3 Kotlin mirror.**
- `android/model/TaskBoard.kt`: `sections`.
- Drop the empty-board filter at `:502-503`.
- Tests (`androidT/model/TaskBoardTest.kt`):
  - `` `every status is a section empty ones included` ``
  - `` `an empty board still has a row` ``

### 3B. Mac board → 3A.1, 3A.2

**3B.1 The list form.** `TaskBoard.swift`:
- `GeometryReader` on the board itself;
- the list form, with collapsed `0` headers that can't expand, and Done and Canceled collapsed by default,
  remembered per workspace;
- the iPhone card row ported;
- the `≡ ▦` segmented toggle in the header.

Test (`MT/BoardFormWiringTests.swift`): `@Test("The board reads its own width, not the window's")`.

**3B.2 New Task…** (ruling 5).
- The `＋` in the header and the empty state's button send `task.create`.
- Tests (`MT/NewTaskTests.swift`):
  - `@Test("New Task files on this workspace's board")`
  - `@Test("A read-only runner offers no New Task")`

**3B.3 The card without a sheet.**
- Extract the card body from the sheet (`TaskBoard.swift:423`, `:855`) into `TaskCard`.
- The sheet stays until slice 2 hosts `TaskCard` in the task column; then it's deleted (task 2D.1).

Test: `@Test("The card shows the question and its Answer buttons in Needs Decision")`, in `MT/TaskCardTests.swift`.

**Slice 3 is done when** today's full-width board, in a narrow window, draws the list, with every status present.

---

## Slice 2: the Mac workspace view → 1D, 3B

### 2A. Measure before fixing (first, blocking 2C)

**2A.1** On a 13" MacBook Air at full screen (1470 pt, and 1440 pt with a scaled display), put the orchestrator,
the board and a task's agent side by side, using a throwaway `HSplitView` branch.
- Record, for claude, codex and cursor, in terminal and in chat mode:
  - the narrowest usable conversation width;
  - the narrowest usable agent width;
  - the width at which a Changes hunk stops wrapping.
- Check what a phone showing the same pane sees when the Mac narrows it (spec R2).
- Write the numbers into the spec's §4.3, replacing 400, 280 and 500 and the 1180 and 680 thresholds.
- No test; the output is the spec edit, which the next tasks read.

### 2B. Selection and migration

**2B.1 The new `Selection` and mapping.**
- `ContentView.swift:138-146`: `Selection` and `Focus` per spec §4.2.
- New `WorkspaceSelection.swift`: `static func mapping(old:, in fleet:) -> Selection`.
- Tests (`MT/WorkspaceSelectionTests.swift`):
  - `@Test("A board selection becomes its workspace")`
  - `@Test("An orchestrator's terminal becomes its workspace with no focus")`
  - `@Test("A dispatched agent's terminal becomes its task")`
  - `@Test("A shell in a claimed worktree becomes that worktree under its owner")`
  - `@Test("A terminal in an unclaimed worktree becomes a loose worktree")`
  - `@Test("A runner without workstreams maps to its repository's implicit workspace")`

**2B.2 Migration.**
- Read `fleet.lastTerminal` (`ContentView.swift:89`) once, write `workspace.lastSelection`, then remove the old key.
- Launch opens Needs You when it has items, else the last selection, replacing `selectFirstRunningTerminal` (`:2100`).
- Tests (`MT/WorkspaceSelectionTests.swift`):
  - `@Test("The old last-terminal key migrates once and is removed")`
  - `@Test("Launch opens Needs You when it has items, else the last workspace")`

### 2C. The three columns → 2A, 2B

**2C.1 The column layout.**
- New `WorkspaceView.swift`: an `HSplitView` of conversation, board and task, with the collapse rules using 2A's
  numbers. The rail closes the task column.
- New `WorkspaceColumns.swift`: `WorkspaceColumns.layout(width:, taskOpen:)`, a pure function.
- Tests (`MT/WorkspaceColumnsTests.swift`):
  - `@Test("All three fit at the measured 13-inch width")`
  - `@Test("Below it, opening a task collapses the conversation to its rail")`
  - `@Test("Below the two-column minimum it's one column with Orchestrator | Board")`

**2C.2 The conversation column.**
- The orchestrator's layout, through `detailFrame` (`WorkspaceSidebar.swift:327-340`); `AgentSurface` in chat
  mode.
- The header menu: Replace Orchestrator…, Show Charter, Terminal / Chat, Restart.
- The empty state with Start Orchestrator (ov-60's `OrchestratorHarness`), and the lost and slow-start states of
  spec §8.
- Tests (`MT/ConversationColumnTests.swift`):
  - `@Test("No orchestrator offers Start Orchestrator with each harness")`
  - `@Test("A lost orchestrator offers Restart and Replace")`
  - `@Test("A start unconfirmed after 30 seconds offers Replace")`
  - `@Test("An orchestrator's finished turn is an unread dot until seen")`

### 2D. The task column → 2C.1, 3B.3

**2D.1 The column.**
- New `TaskColumn.swift`:
  - the header, with the status pop-up, agent picker, Open Worktree and close;
  - `TaskCard` as the expanding header, which starts expanded in Needs Decision;
  - the agent's layout via `tiled` (`ContentView.swift:1813-1939`);
  - `ChangesPane(changes: changesStore(for:client:), isFocused:, agents: reviewTargets)`, embedded directly, as
    `TileView.swift:416` draws it, with no tmux split (ruling 6).
- Delete the card sheet.
- Tests (`MT/TaskColumnTests.swift`):
  - `@Test("A task in review leads with its changes")`
  - `@Test("A task with no agent but a worktree offers Open Worktree")`
  - `@Test("A task with neither says nothing has started")`
  - `@Test("Showing changes in the column opens no tmux pane")`: asserts no `split` call is made.

**2D.2 Open Worktree and Focus.**
- The breadcrumb, Esc and ⌘[ to go back, and ⇧⌘↩ to focus.
- Test: `@Test("Open Worktree shows the worktree's own layouts, and Esc returns to the task")`.

### 2E. Sidebar, Needs You, palette and commands → 2B.1 (∥ 2C and 2D)

**2E.1 The sidebar.** `WorkspaceSidebar.swift` and `SidebarViews.swift`:
- the Needs You row;
- workspace rows with the orchestrator glyph and count;
- the Worktrees disclosure;
- the Board and Orchestrator rows removed;
- the header badge removed;
- the sidebar at 220, 248 and 360 (`ContentView.swift:1205`).

Tests (`MT/BoardSidebarTests.swift`, extending the existing file):
- `@Test("A workspace row counts its items")`
- `@Test("The Worktrees disclosure lists the workspace's worktrees with their task keys")`
- `@Test("The Worktrees disclosure opens itself when the selection is inside it")`

**2E.2 Move to Workspace ▸** (ruling 5): a menu on a worktree row, sending `worktree.assign`.
Tests (`MT/WorktreeDragGateTests.swift`): `@Test("Move to Workspace offers the same targets the drag accepts")`.

**2E.3 Needs You view.**
- New `NeedsYouView.swift`: rows with in-place buttons.
  - A refusal reads "Someone already answered this." or "Couldn't reach claude. Try again."
  - Review rows have only a Review button (ruling 2).
- Tests (`MT/NeedsYouViewTests.swift`):
  - `@Test("An ask answers in place and a refusal stays with its line")`
  - `@Test("A review can be opened, not approved")`
  - `@Test("A decision's options are its buttons")`

**2E.4 Palette and commands.**
- `PaletteIndex.swift:10-21`: `.openWorkspace` and `.openTask`, with `newTask` renamed `newWorktree`.
- `Commands.swift`: the Find label, ⇧⌘B, and ⌥⌘1, ⌥⌘2 and ⌥⌘3.
- Window titles.
- Copy fixes: "Primary checkout" becomes "Main checkout" (`SidebarViews.swift:208`); "project" goes
  (`Shortcuts.swift:115`, `QuickCreate.swift:160`, `ContentView.swift:2693`).
- Tests:
  - `MT/PaletteWorkspaceTests.swift`:
    - `@Test("Typing a task key finds the task")`
    - `@Test("A workspace with no matching worktree is still found")`
  - `MT/ShortcutSheetTests.swift`: `@Test("No shortcut copy says project")`

### 2F. First-launch tip → 2E.1

**2F.1** A one-time tip, stored under `tips.workspaces`.
Test: `@Test("The tip shows once")`.

**Slice 2 is done when**, on a full-screen 13" Mac, a workspace shows its orchestrator and board, a task opens its
agent and changes, and the Needs You row answers an ask. Check this by driving the real app, with the live app's
process left alone.

---

## Slice 4: the phones → 1C, 1D, 1E, 3A

### 4A. iOS (one lane)

**4A.1 The root stack.**
- `ios/FarCoolerApp.swift`: a `NavigationStack`.
- New `ios/NeedsYouScreen.swift`: items, then the Workspaces sections.
- Launch opens Needs You when it has items, else the last workspace, pushed over it (ruling 4).
- Tests (`AKT/ShellNavigationTests.swift`, extended):
  - `@Test("Launch lands on Needs You when it has items")`
  - `@Test("With nothing waiting, launch pushes the last workspace over Needs You")`

**4A.2 The workspace screen.**
- New `ios/WorkspaceScreen.swift`: a segmented Orchestrator, Board and Worktrees control, remembered per
  workspace.
- The board is in-line, from `sections`. Retire `BoardSheetHost` (`ShellScreen.swift:1150-1166`, `:2259`).
- Start Orchestrator, Restart and Replace (ruling 8).
- New Worktree… claims for this workspace (1C.3).
- UI tests: `apps/ios/FarCoolerUITests/WorkspaceScreenTests.swift`:
  - `testWorkspaceShowsOrchestratorBoardAndWorktrees`
  - `testStartOrchestratorFromAnEmptyWorkspace`

  Run them through `scripts/ios-ui-tests.sh`, never bare `xcodebuild`. Replace `ShellBoardTests.swift`.

**4A.3 Task and worktree screens.**
- New `ios/TaskScreen.swift`: the card with Answer buttons, then Agent, Changes and Worktree rows that push.
- The worktree screen is today's shell, scoped to one worktree:
  - the sideways swipe stays within it;
  - the overview retires;
  - "Diff" becomes "Changes" (`ShellScreen.swift:294`).
- Deep links push the workspace, then the task, then the agent.
- UI tests:
  - `testGoingToATasksAgentAndBackReturnsToTheTask`
  - `testANotificationTapLandsWithTheWorkspaceUnderIt`

**4A.4 Vocabulary.**
- "Quick Task" becomes "New Worktree…".
- "Project" becomes "Repository" (`TaskComposer.swift:91-92`).
- The byline "The manager" becomes "Orchestrator" (`AK/TaskBoardModel.swift:731`).
- Migrate the `quicktask.*` drafts.

Test: `AKT/TaskBoardModelTests.swift` `@Test("An orchestrator's note is bylined Orchestrator")`.

### 4B. Android (one lane, ∥ 4A)

**4B.1 Routes.**
- `android/ui/Navigation.kt`: `Workspace(host, workspace, tab)`; `Board` decodes as
  `Workspace(tab = board)`; `Fleet` retires.
- Tests (`androidT/ui/NavigationTest.kt`):
  - `` `an old saved Board route restores as the workspace's board tab` ``
  - `` `an old saved Fleet route is dropped and the stack still restores` ``
  - `` `back from a task's agent returns to the task` ``

**4B.2 Needs You.**
- `android/model/NeedsYou.kt` and `ui/NeedsYouScreen.kt` render merged items with workspace labels.
- Drop the worktree sections and the Board band (`NeedsYouScreen.kt:276-280`).
- Tests (`androidT/model/NeedsYouTest.kt`, rewritten):
  - `` `items are labeled by workspace and ordered by rank` ``
  - `` `a decision is an item, so nothing needs you is never shown above one` ``
  - `` `a finished agent is not an item` ``

**4B.3 Workspace, task and worktree screens.**
- New `ui/WorkspaceScreen.kt`: a `TabRow`.
- `BoardScreen.kt` becomes the Board tab, using `sections`.
- The task screen gains Changes and Worktree rows. This is the first reader of `TaskRow.worktreeId`.
- The worktree screen gets a back arrow when it's pushed.
- Start Orchestrator; New Worktree claims for the workspace.
- Vocabulary: "Quick task", "Main board" and "Primary checkout" (`FleetScreen.kt:332`, `BoardScreen.kt:116`,
  `NeedsYouScreen.kt:404`).
- Tests:
  - `` `a task with a worktree but no agent reaches its changes` ``
  - `` `the board tab's title is Board` ``

### 4C. Glances (one lane, ∥ 4A and 4B) → 1B

**4C.1 Widget.**
- `AK/FleetSnapshot.swift` gains optional `needsYou`, written by the app.
- The widget's count and rows come from it.
- Tests (`AKT/FleetSnapshotTests.swift`):
  - `@Test("A snapshot without needsYou decodes as before")`
  - `@Test("The widget's count is the item count")`

**4C.2 Live Activity.**
- The header uses the relay's `needsYou`.
- Rows are labeled "Billing · bil-9".
- Test (`AKT/AgentCardRowsTests.swift`): `@Test("The header counts needs-you items when the relay sends them")`.

**4C.3 Watch.**
- A Needs You section first.
- Asks answer in place; decisions and reviews show "Open on iPhone".
- The complication shows the count.

Test: `@Test("The watch lists items before agents")`, in `AKT/`.

**Slice 4 is done when** both phones open on Needs You, answer an ask, go workspace → task → agent → back to task,
and the lock screen's count matches the app's.

---

## Parallel map

```
day one ─┬─ 1A daemon ──────┬─ 1C client core + CLI ─┬─ 1D AgentKit ─┬─ 2B ─┬─ 2C ─ 2D ─┐
         │                  │                        │               │      └─ 2E ─ 2F ─┤
         ├─ 1B relay ───────┼────────────────────────┼───────────────┼─ 4C glances ─────┤
         │                  │                        └─ 1E Kotlin ───┼─ 4B Android ─────┤
         └─ 3A shared rules ┴─ 3B Mac board ─────────────────────────┼─ 4A iOS ─────────┤
                                                   2A measurement ───┘ (before 2C)      └─ done
```

**Rules for the lanes:**
- 2A, the measurement, can run any time before 2C, including on day one.
- 3B and slice 2 both edit `TaskBoard.swift`, so 3B lands first.
- 2C/2D and 2E edit different files, apart from `ContentView.swift`'s `detail` and `sidebar`. They rebase on each
  other.
- The coordinator records each lane on the board: `farcooler-canary task … --repo overnight`.
