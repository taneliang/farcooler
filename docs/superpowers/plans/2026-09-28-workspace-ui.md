# The workspace UI: implementation plan

Date: 2026-09-28 (revised after the pre-flight review, `.claude/agent/reports/workspace-ui-preflight.md`)
Spec: [`docs/superpowers/specs/2026-09-28-workspace-ui-design.md`](../specs/2026-09-28-workspace-ui-design.md) (decided; its rulings are in §12)
Status: not started

## How to read this

**Paths:**
- Mac paths are relative to `apps/macos/Sources/FarCooler/`.
- `AK/` is `apps/shared/AgentKit/Sources/AgentKit/`, and `AKT/` is its `Tests/AgentKitTests/`.
- `MT/` is `apps/macos/Tests/CeremonyTests/`.
- `android/` is `apps/android/app/src/main/java/com/farcooler/`, and `androidT/` is its `src/test` twin.

**Test style per platform:**
- Rust tests are named as sentences.
- Swift uses Swift Testing: `@Test("…")`.
- Kotlin uses backtick names.
- The relay uses `it('…')`.

**Every test named here is one that a wrong implementation makes fail.** Each is broken once on purpose and seen to
go red before its task is done, and the commit message says how it was broken. A test that is green before its
task starts is a defect in this plan.

**Other rules:**
- The Rust tree is hand-formatted. Never run `cargo fmt`. Cargo is off PATH.
- No merge commits. A lane is rebased or squashed onto `main`.
- Scratch daemons get a short `FARCOOLER_HOME` and are killed by PID.

**File ownership is the parallel rule.** Each shared file below has exactly one owning task at a time. A task
that needs a change in a file it doesn't own gets that change from the owner, which is listed as its dependency.

| File | Owner, in order |
|---|---|
| `proto/farcooler.proto`, `crates/protocol/src/lib.rs` | 1A.1 |
| `crates/daemon/**` | 1A |
| `crates/client/src/session.rs`, `ffi.rs` | 1A.1 (the event arms only), then 1C |
| `crates/cli/src/main.rs` | 1A.1 (the `event_json` arm only), then 1C.2 |
| `services/relay/**` | 1B |
| `AK/RunnerBoards.swift`, `AK/BoardForm.swift`, Kotlin `model/TaskBoard.kt` | 3A |
| `AK/NeedsYou.swift`, `AK/TaskLink.swift`, `AK/CoreModel.swift` | 1D |
| Kotlin `model/NeedsYouItems.kt`, `TaskLink.kt`, `Model.kt` | 1E |
| `TaskBoard.swift`, `DaemonClient.swift` (task writes) | 3B, then slice 2 |
| All other Mac app files | slice 2 (one lane, in order) |
| `apps/ios/FarCooler/**` (except the three below), `AK/ShellNavigation.swift`, `AK/TaskBoardModel.swift` | 4A |
| `ios/FleetSnapshotWriter.swift`, `ios/WatchLinkHost.swift`, `AK/FleetSnapshot.swift`, `AK/AgentCardRows.swift`, the widget, Live Activity and watch targets | 4C |
| `android/ui/**`, `android/model/NeedsYou.kt` | 4B (after 1E.2) |

---

## Final parallel groups

```
Group 1 (day one):     [1A daemon]   [1B relay]   [3A shared board rules → 3B Mac board]   [2A measurement]
Group 2 (after 1A):    [1C client core + CLI]
Group 3 (after 1C):    [1D AgentKit]   [1E Kotlin]
Group 4 (after 1D, 1E, 3A, 3B, 2A):
                       [slice 2: Mac, one lane]   [4A iOS]   [4B Android]   [4C glances, after 4A.1]
Group 5 (after 4A, 4B): [4D delete `listed`]
```

- **Slices 1 and 3 start together and share no file.** 3A adds `sections` beside `listed` and deletes nothing, so
  it doesn't break either phone.
- 1D and 3A both edit the AgentKit package, in different files. Each runs
  `swift test --package-path apps/shared/AgentKit` on the rebased tree before landing.
- **Slice 2 is one lane,** because 2B–2F all touch `ContentView.swift`, `WorkspaceSidebar.swift` or `Commands.swift`.
  Only 2A runs apart from it.
- 4C starts after 4A.1, which adds the per-runner needs-you store to `ios/FleetStore.swift`. After that, 4A and 4C
  own disjoint files.

---

## Slice 1: the rollup and the links

### 1A. Daemon (one lane, in order)

**1A.1 Proto, capability, and the arms that keep every crate compiling** (one commit).
- `proto/farcooler.proto`:
  - `enum NeedsYouKind { NEEDS_YOU_KIND_UNSPECIFIED = 0; NEEDS_YOU_KIND_ASK = 1; NEEDS_YOU_KIND_BLOCKED = 2; NEEDS_YOU_KIND_DECISION = 3; NEEDS_YOU_KIND_REVIEW = 4; }`
  - `NeedsYouItem`, `NeedsYouAction`, `TaskRef`, `TerminalRef`, `WorktreeRef` and `NeedsYouList`, per spec §2.4;
  - `Event.payload`: `Empty needs_you_changed = 24`;
  - `Result.value`: `NeedsYouList needs_you_list = 45` (27–31 stay unused);
  - `Worktree`: `repeated TaskRef open_tasks = 14`.
- `crates/protocol/src/lib.rs`:
  - `capability::NEEDS_YOU = "needs_you"`, added to `ALL` (`:394-399`);
  - `"needs_you.list" => NEEDS_YOU` in `for_method`;
  - `needs_you.list` added to `every_capability_a_method_names_is_one_this_build_advertises` (`:843`), **not** to
    `workstreams_is_not_the_frozen_worktrees_word` (`:790`);
  - `open_tasks = 14` added to the whole-message `Worktree` table in `worktree_rename…`.
- `crates/client/src/session.rs`: `FleetEvent::NeedsYou`, an arm in `FleetEvent::of` (`:264-318`, which is
  exhaustive on purpose).
- `crates/client/src/ffi.rs`: the `event_line` arm `{"event":"needs_you"}` (`:222`).
- `crates/cli/src/main.rs`: the `event_json` arm `{"kind":"needs_you"}` (before `_ => return None`, `:3581`), and
  its row in `every_resource_a_client_reads_gets_a_line_rather_than_being_swept_up` (`:4343`).
- Tests:
  - `needs_you_takes_fresh_tags`, a sibling of `workspace_additions_take_fresh_tags` (`lib.rs:739`), pinning 24, 45
    and 14;
  - `needs_you_list_is_gated_on_its_own_advertised_capability`;
  - `a_needs_you_event_is_a_fleet_event` (in `session.rs`), and the `event_line` case in `ffi.rs`'s tests.

**1A.2a Accessors for open asks** (review B1).
- `crates/daemon/src/hook_asks.rs`:
  - `hold` records a wall-clock `since: SystemTime` beside the `Instant`;
  - new `pub fn open(&self) -> Vec<(Uuid, String, SystemTime)>` (terminal, id, since).
- `crates/daemon/src/agent_supervisor.rs`: new
  `pub fn open_permission(&self, terminal: Uuid) -> Option<(String, Vec<PermissionOption>, SystemTime)>`, the last
  `Permission` in the terminal's `recent` ring with no later `Resolved` of the same id. Hook asks reach the same ring
  through the sink, so this one accessor serves both sources.
- Tests:
  - `open_lists_a_held_ask_and_forgets_it_once_settled` (`hook_asks.rs`)
  - `a_newer_ask_on_the_same_terminal_replaces_the_older_in_open`
  - `open_permission_is_none_once_its_resolved_arrives` (`agent_supervisor.rs`)
  - `open_permission_returns_the_later_of_two_unresolved_asks`

**1A.2b Item assembly, as a pure function.**
- New `crates/daemon/src/needs_you.rs`: `pub fn assemble(inputs: &Inputs, now: SystemTime) -> Vec<pb::NeedsYouItem>`.
  `Inputs` holds:
  - a watcher snapshot of `Observed` (activity, `blocked_question`, `turn_failed`, state age) via a new
    `Watcher::observed_snapshot()`;
  - terminals from the store (`task_id`, `workspace_id`, `role`, `worktree_id`);
  - `HookAsks::open` and `open_permission`;
  - `Store::list_tasks(scope, Some(status))` for Needs Decision and In Review (`store/tasks.rs:455`);
  - `notes_for(task, …)` for the latest `QUESTION`, its options (`extra_json.options`, `cli/tasks.rs:734`), and any
    later `ANSWER`;
  - worktree owners and hidden flags, and workspace names.
- `pub fn redact_below_control(item) -> item`, per spec §2.4's ruling. This is the only place the Read shape is
  made.
- Tests (`needs_you.rs` `mod tests`):
  - `a_task_with_a_held_ask_and_a_decision_is_one_ask_item_with_decision_in_also`
  - `an_ask_outranks_a_block_outranks_a_decision_outranks_a_review`
  - `within_a_kind_the_oldest_comes_first`
  - `a_done_agent_is_not_an_item`
  - `an_orchestrators_finished_turn_is_not_an_item`
  - `an_orchestrators_ask_is_an_item_about_its_own_terminal`
  - `a_blocked_codex_with_no_open_ask_is_a_blocked_item`
  - `a_failed_turn_is_a_blocked_item_and_a_seen_one_is_not` (seen: activity Idle)
  - `a_bad_exit_is_not_an_item`
  - `an_answered_decision_is_not_an_item`: an `ANSWER` after the `QUESTION`, status still Needs Decision
  - `a_task_in_review_is_a_review_item_with_only_an_open_action`
  - `an_item_takes_the_tasks_workspace_then_the_terminals_then_the_worktree_owners`
  - `an_item_in_a_hidden_worktree_still_counts`
  - `an_unclaimed_worktrees_item_has_an_empty_workspace`
  - `a_decisions_options_become_its_actions`
  - `an_items_id_survives_its_rank_changing`: assemble at `now` and at `now + 10 min`; the ranks differ and the ids
    don't
  - `a_superseding_ask_changes_the_items_id`
  - `a_read_scoped_item_carries_no_ask_command_path_or_actions`: a hook ask whose allow option is
    "Allow /tmp/probe/x.txt"; nothing in the redacted item contains `/tmp/probe`

**1A.3 The call and the refusal words.**
- `crates/daemon/src/rpc.rs`:
  - `"needs_you.list" => Scope::Read`;
  - a dispatch arm that assembles, then redacts when the peer's scope is below Control.
- `terminal.agent_answer` (`:2105-2110`): `NotHeld` → `ResourceConflict` with `what: "not_held"`, and
  `NotDelivered` → `what: "not_delivered"`.
- Tests:
  - New `crates/daemon/tests/needs_you_reads_every_kind_over_the_socket.rs`, on `rpc_over_socket.rs`'s harness:
    - `every_kind_comes_back_in_rank_order`
    - `a_read_scoped_client_gets_the_redacted_shape`
  - `crates/daemon/src/rpc.rs` tests:
    - `a_second_answer_is_refused_as_not_held`
    - `an_undelivered_answer_is_refused_as_not_delivered`

**1A.4 The event, debounced.**
- A `needs_you_changed` debounce in the watcher: 250 ms, trailing edge, a new `announce_needs_you`.
- It's called from every trigger in spec §2.4, including a `QUESTION` or `ANSWER` note on a task in Needs Decision.
- Tests: new `crates/daemon/tests/needs_you_changed_is_announced.rs`, which subscribes to events over the socket.
  Each test is broken by deleting its one `announce_needs_you` call.
  - `settling_a_held_ask_announces_needs_you_changed`
  - `moving_a_task_into_needs_decision_announces_needs_you_changed`
  - `answering_a_decision_announces_needs_you_changed`
  - `seeing_a_failed_turn_announces_needs_you_changed`
  - `ten_changes_inside_the_window_announce_once`

**1A.5 `Worktree.open_tasks`.**
- Join the tasks onto `WorktreeView` where it's built, since `wire::worktree` (`wire.rs:189`) has no store.
- Emit `worktree_changed` on every trigger in spec §3.2 item 1:
  - `task.update`, both lanes, and a title change;
  - `task.create` with a worktree;
  - `task.set_status` into or out of Done or Cancelled.
- Tests:
  - `a_worktree_lists_its_open_tasks_and_not_its_done_ones`
  - `moving_a_tasks_lane_changes_both_worktrees`
  - `finishing_a_task_changes_its_worktree`
  - `renaming_a_task_changes_its_worktree`

**1A.6 Push: the pinned contract** (spec §7).
- `crates/daemon/src/push.rs`:
  - `Notification` gains `workspace: Option<&str>` and `#[serde(rename = "needsYou")] needs_you: Option<u32>`;
  - `kind: Option<&str>` for `"decision"` and `"count"`;
  - `Outgoing.terminal` becomes `Option`, required when `kind` is absent.
- `crates/daemon/src/watch.rs`:
  - The title leads with the workspace.
  - A decision notice goes out when a task enters Needs Decision. It has no terminal.
  - A count notice, debounced to 2 s, goes out after a `needs_you_changed` that no other notice carried.
  - The count comes from the same `assemble` call.
- Tests (`watch.rs` and `push.rs` `mod tests`):
  - `a_blocked_agents_notice_leads_with_its_workspace`
  - `an_orchestrators_notice_is_its_workspace_alone`
  - `a_decision_notice_has_a_task_and_no_terminal`
  - `answering_a_chat_ask_sends_a_count_notice`
  - `the_push_count_is_the_lists_length`: both come from one `assemble`; broken by counting terminals instead
  - `the_body_spells_needs_you_in_camel_case`

### 1B. Relay (one lane, ∥ 1A; the contract is spec §7)

**1B.1 Storage and the sum.**
- New `services/relay/migrations/0010_needs_you.sql`: nullable `needs_you INTEGER` and `needs_you_at INTEGER` on
  `daemons`.
- `services/relay/src/index.ts`:
  - Overwrite both on any notice carrying `needsYou`.
  - `kind: "decision"` alerts and writes no `live_activities` row.
  - `kind: "count"` updates the card silently.
  - `readFleet`'s purge nulls counts older than `ROW_RETENTION_MS`.
- `services/relay/src/push.ts`: the content-state gains `needsYou`, the sum over unrevoked daemons with a fresh
  count. With none, the header falls back to `blocked`.
- Tests (`services/relay/test/relay.test.ts`):
  - `it('sums the latest needs-you count per daemon')`
  - `it('replaces a daemon\'s count rather than adding to it')`
  - `it('drops a count older than a day from the sum')`
  - `it('never sums a revoked daemon\'s count')`
  - `it('falls back to the blocked count when no daemon sent one')`
  - `it('alerts on a decision notice and writes no roster row')`
  - `it('updates the card on a count notice without alerting')`

### 1C. Client core and CLI (one lane, → 1A; owns `session.rs`, `ffi.rs` and `cli/main.rs` from here)

**1C.1 The JSON projection.**
- New `crates/client/src/needs_you_json.rs`: `needs_you_json(&pb::NeedsYouList) -> Value`, in **snake_case** keys,
  following `inbox_json`.
- `Session::needs_you`, and `"needs_you"` in the FFI call table.
- `open_tasks` in `Session::fleet`'s worktree JSON.
- Write `test/fixtures/needs-you.json`: two runners, every kind, and every optional field set.
- Tests:
  - `every_needs_you_item_field_has_a_json_key`: walks the `NeedsYouItem` descriptor, as `worktree_rename…` does,
    so a field added later without a key fails
  - `the_shared_fixture_is_what_needs_you_json_writes`
  - `the_shared_fixture_sets_every_optional_field`: every optional is set and every repeated field is non-empty, so
    the decode tests can't pass vacuously

**1C.2 The CLI.** `crates/cli/src/main.rs`:
- `farcooler needs-you [--json]`;
- `open_tasks` in `worktree list --json`;
- the error JSON carries the refusal's `what`.

Tests:
- `needs_you_prints_one_line_per_item_in_rank_order`
- `needs_you_json_is_the_client_cores_shape`
- `needs_you_on_a_runner_without_the_capability_says_to_update_it`
- `a_refusals_what_reaches_the_error_json`

**1C.3 The phone bridge** (`ffi.rs` and `session.rs`; review §6).
- `"task.note"`: task, kind and body, for answering a decision.
- `"workspace.start_orchestrator"`: workspace, harness and replace (ruling 8).
- `"terminal.restart"`, if it isn't already reachable.
- `"worktree.create"` takes an optional `workspace`, and `main_claim` (`session.rs:1024`) falls back to Main
  (ruling 8).
- Tests (`ffi.rs` `mod tests`):
  - `task_note_sends_an_answer_as_the_user`
  - `start_orchestrator_sends_its_harness_and_replace`
  - `a_worktree_made_with_a_workspace_is_claimed_for_it`
  - `a_worktree_made_without_one_is_still_claimed_for_main`

### 1D. AgentKit (→ 1C; ∥ 1E)

**1D.1 Decode, merge, and the older-runner fallback.**
- New `AK/NeedsYou.swift`:
  - `NeedsYouItem`;
  - `NeedsYou.merge(_:)`, by rank, then runner;
  - `count(in:)`;
  - `NeedsYou.derived(fromTerminals:)` for a runner without `needs_you` (spec §2.6), re-tiered into blocked.
- Tests (`AKT/NeedsYouTests.swift`):
  - `@Test("The shared fixture decodes to the values it holds")`: asserts each field's value, reading
    `test/fixtures/needs-you.json` via `#filePath`
  - `@Test("Two runners merge by rank, not by clock")`
  - `@Test("An unknown kind decodes and sorts last")`
  - `@Test("A workspace's count is its items, not its signals")`
  - `@Test("An older runner's blocked agent is a blocked item, never above a real ask")`
  - `@Test("An older runner derives no decisions or reviews")`

**1D.2 The pane's task.**
- `AK/CoreModel.swift` decodes `openTasks`.
- New `AK/TaskLink.swift`:
  - protocols `TaskLinkPane` (`boardTaskID`) and `TaskLinkWorktree` (`openTaskIDs`), so the Mac's own types conform,
    as they do for `TaskBoardPane`;
  - `TaskLink.task(of:in:)`.
- Tests (`AKT/TaskLinkTests.swift`):
  - `@Test("A dispatched pane's task is its own, over its worktree's")`
  - `@Test("A pane in a worktree with one open task shows that task")`
  - `@Test("A pane in a worktree with two open tasks shows none")`
  - `@Test("A shell shown under a task is not counted as that task's agent")`: a worktree with one open task and a
    shell with no `taskId`. `TaskLink` returns the task, **and** `tasksWithLiveAgents` for that board is 0

### 1E. Kotlin (→ 1C; ∥ 1D)

**1E.1** Mirror 1D in new `android/model/NeedsYouItems.kt` and `TaskLink.kt`, and decode `openTasks` in `Model.kt`.
- Tests:
  - `androidT/model/NeedsYouItemsTest.kt`:
    - `` `the shared fixture decodes to the values it holds` ``
    - `` `two runners merge by rank not by clock` ``
    - `` `an older runner's blocked agent never outranks a real ask` ``
  - `androidT/model/TaskLinkTest.kt`:
    - `` `one open task is the pane's task` ``
    - `` `two open tasks are none` ``
    - `` `a shell under a task is not its agent` ``

**1E.2** A task chip in `android/ui/TerminalPane.kt`'s top bar (`:408-452`).
- Test: `` `the top bar names the pane's task` `` (`androidT/ui/`).

**Slice 1, usable alone:**
- `farcooler needs-you` on any runner;
- the lock screen's count covers decisions;
- Android's panes name their task.

---

## Slice 3: the responsive board (∥ slice 1 from day one)

### 3A. Shared rules (one lane: AgentKit, then Kotlin)

**3A.1 `sections` beside `listed`.**
- `AK/RunnerBoards.swift`: `TaskBoardModel.sections`, every status in `order` with its count. Keep `listed`, marked
  deprecated. It's deleted in 4D.
- An implicit board gets a row when empty (spec §5; `:170` hides it today).
- Tests (`AKT/RunnerBoardsTests.swift`):
  - `@Test("Every status is a section, empty ones included")`
  - `@Test("An empty implicit board still has a row")`: red today

**3A.2 The form rule.** New `AK/BoardForm.swift`:
- `resolve(width:previous:forced:)`: a list below 800, a kanban at 824 and up, and between the two, the form it
  already has;
- `Choice` (`auto | list | kanban`), stored per device under `board.form.<host>.<workspace>`.

Tests (`AKT/BoardFormTests.swift`):
- `@Test("Below 800 it's a list, at 824 and up a kanban")`
- `@Test("Between 800 and 824 it keeps the form it had")`
- `@Test("A forced form ignores width")`
- `@Test("Choosing the forced form again returns to Automatic")`

**3A.3 Kotlin.** `android/model/TaskBoard.kt`:
- `sections` beside `listed` (`:229`);
- the implicit-board row (`:508`).

Tests (`androidT/model/TaskBoardTest.kt`):
- `` `every status is a section empty ones included` ``
- `` `an empty implicit board still has a row` ``

### 3B. Mac board (→ 3A.1, 3A.2; owns `TaskBoard.swift` and `DaemonClient.swift`'s task writes)

**3B.1 The list form.** `TaskBoard.swift`:
- `GeometryReader` on the board's own body;
- the list form, with collapsed `0` headers that can't expand, and Done and Canceled collapsed by default,
  remembered per workspace;
- a Mac card row written after the iPhone's design (`ios/TaskBoardView.swift` isn't touched; 4A owns it);
- the `≡ ▦` toggle.

Test (`MT/BoardFormWiringTests.swift`): `@Test("A 600-pt board in a 1600-pt window draws the list")`. It hosts
`TaskBoardView` at a 600 pt frame inside an `NSHostingView` 1600 pt wide and finds the list's accessibility
identifier. It fails if the form is read from the window.

**3B.2 Task writes.**
- `DaemonClient.swift`: `createTask(title:workspace:repository:)` and `answerDecision(key:body:repository:)`, the
  latter as `task note --kind answer`.
- New Task… in the header and in the empty state (ruling 5).
- Tests, in `MT/WorktreeCallsTests.swift` with its `Recorder`, which checks each line against the real CLI's
  `--help`:
  - `@Test("New Task sends task create on this workspace's board")`
  - `@Test("Answering a decision sends task note as an answer")`
- Test (`MT/NewTaskTests.swift`): `@Test("A read-only runner offers no New Task")`

**3B.3 `TaskCard`.**
- Extract the card body from the sheet (`TaskBoard.swift:423`, `:855`). The sheet hosts it until 2D.1.
- The card's Answer buttons use 3B.2.
- Test (`MT/TaskCardTests.swift`): `@Test("A card in Needs Decision shows its question's options as buttons")`.

**Slice 3, usable alone:** today's full-width board, in a narrow window, draws the list with every status present,
and New Task works.

---

## Slice 2: the Mac workspace view (one lane, in order)

### 2A. Measurement (∥ everything; day one)

**2A.1** Measure on a 13" MacBook Air at full screen (1470 pt, and 1440 pt scaled), with a throwaway `HSplitView`
branch.
- Put the orchestrator, the board and a task's agent side by side.
- Record, for claude, codex and cursor, in terminal and chat mode:
  - the narrowest usable conversation width;
  - the narrowest usable agent width;
  - the width at which a Changes hunk stops wrapping.
- Check what a phone showing the same pane sees when the Mac narrows it (spec R2).
- Write the numbers into spec §4.3, replacing 400, 280 and 500, and the 1180 and 680 thresholds.
- The output is the spec edit, which 2C's tests read. There's no test.

### 2B. Data and selection (→ 1D, 3B)

**2B.1 Mac data.**
- `DaemonClient.swift`:
  - read `farcooler needs-you --json`, re-reading on the `needs_you` event line;
  - on a runner without the capability, use `NeedsYou.derived`;
  - add `answerAsk(terminal:request:option:)` (`terminal agent-answer`).
- `Model.swift` decodes `open_tasks`.
- `Terminal` and `Worktree` conform to `TaskLinkPane` and `TaskLinkWorktree`.
- The header badge (`ContentView.swift:1280-1293`) and ⌃⌘N (`:2656-2663`) use the merged items.
- Tests:
  - `MT/WorktreeCallsTests.swift`:
    - `@Test("Needs You is read with farcooler needs-you --json")`
    - `@Test("An ask is answered with terminal agent-answer")`
  - `MT/NeedsYouCycleTests.swift`:
    - `@Test("⌃⌘N reaches a decision")`
    - `@Test("⌃⌘N skips a finished agent")`

**2B.2 `Selection` and its mapping.**
- `ContentView.swift:138-146`: the new `Selection` and `Focus`.
- New `WorkspaceSelection.swift`: `mapping(old:in:)`.
- Tests (`MT/WorkspaceSelectionTests.swift`):
  - `@Test("A board selection becomes its workspace")`
  - `@Test("An orchestrator's terminal becomes its workspace with no focus")`
  - `@Test("A dispatched agent's terminal becomes its task")`
  - `@Test("A shell in a claimed worktree becomes that worktree under its owner")`
  - `@Test("A terminal in an unclaimed worktree becomes a loose worktree")`
  - `@Test("A runner without workstreams maps to its repository's implicit workspace")`

**2B.3 Migration and launch.**
- `fleet.lastTerminal` (`ContentView.swift:89`) becomes `workspace.lastSelection`, once.
- Launch opens Needs You when it has items, else the last workspace, replacing `selectFirstRunningTerminal`
  (`:2100`).
- Tests:
  - `@Test("The old last-terminal key migrates once and is removed")`
  - `@Test("Launch opens Needs You when it has items, else the last workspace")`

### 2C. The columns (→ 2A, 2B)

**2C.1** New `WorkspaceColumns.swift`: `layout(width:taskOpen:)`, a pure function, using 2A's numbers. New
`WorkspaceView.swift`: an `HSplitView` of conversation, board and task, with the rail.
- Tests (`MT/WorkspaceColumnsTests.swift`), each asserting against the measured constants:
  - `@Test("All three fit at the measured full-screen 13-inch width")`
  - `@Test("Below it, opening a task collapses the conversation to its rail")`
  - `@Test("Below the two-column minimum it's one column with Orchestrator | Board")`

**2C.2** The conversation column: the orchestrator via `detailFrame` (`WorkspaceSidebar.swift:327-340`), chat in agent
mode, the header menu, and spec §8's states.
- Tests (`MT/ConversationColumnTests.swift`):
  - `@Test("No orchestrator offers Start Orchestrator with each harness")`
  - `@Test("A lost orchestrator offers Restart and Replace")`
  - `@Test("A start unconfirmed after 30 seconds offers Replace")`
  - `@Test("An orchestrator's finished turn is an unread dot until seen")`

### 2D. The task column (→ 2C, 3B.3)

**2D.1** New `TaskColumn.swift`:
- the header, with the status pop-up (replacing Move To), the agent picker, Open Worktree and close;
- `TaskCard`;
- the agent's layout via `tiled`;
- `ChangesPane(changes: changesStore(for:client:), isFocused:, agents: reviewTargets)`, embedded directly (ruling 6);
- the divider, remembered per window;
- delete the card sheet.

Tests (`MT/TaskColumnTests.swift`):
- `@Test("A task in review leads with its changes")`
- `@Test("A task with no agent but a worktree offers Open Worktree")`
- `@Test("A task with neither says nothing has started")`
- `@Test("Opening a task's changes runs no split, and the toolbar's Changes button does")`: both against the
  `WorktreeCallsTests` `Recorder`. The second half is the control that shows the recorder sees a split.

**2D.2** Open Worktree: the breadcrumb and Back.
- Test: `@Test("Open Worktree shows the worktree's own layouts, and Back returns to the task")`.

### 2E. Sidebar, Needs You, palette and commands (→ 2D; this task owns every `Commands.swift` change)

**2E.1 The sidebar.** `WorkspaceSidebar.swift` and `SidebarViews.swift`:
- the Needs You row;
- workspace rows with the orchestrator glyph and count, carrying ov-60's header menu and the drop target;
- the Worktrees disclosure (`sidebar.openWorktrees`);
- delete the top-level worktree rows under a repository;
- delete the Board and Orchestrator rows, and the header badge;
- the sidebar at 220, 248 and 360.

Tests (`MT/BoardSidebarTests.swift`):
- `@Test("A workspace row counts its items")`
- `@Test("A repository lists workspaces, not worktrees")`
- `@Test("The Worktrees disclosure lists the workspace's worktrees with their open task keys")`
- `@Test("A workspace row offers Show Board, Start Orchestrator and Show Charter")`

**2E.2 Move to Workspace ▸** (ruling 5).
- Test (`MT/WorktreeDragGateTests.swift`): `@Test("Move to Workspace offers exactly the targets the drag accepts")`.

**2E.3 The Needs You view.**
- New `NeedsYouView.swift`: in-place buttons; the refusal lines chosen by `what`; Review only on review rows; no
  buttons below Control.
- Tests (`MT/NeedsYouViewTests.swift`):
  - `@Test("A not_held refusal says someone already answered, and not_delivered says try again")`
  - `@Test("A review row can be opened, not approved")`
  - `@Test("A read-only runner's items have only Open")`

**2E.4 Palette, commands and copy.**
- `PaletteIndex.swift`: `.openWorkspace`, `.openTask`, orchestrator rows, and `newTask` renamed `newWorktree`.
- `Commands.swift`:
  - Back ⌃⌘←, Focus Column ⌃⌘↩, and ⌥⌘1–3;
  - ⇧⌘B;
  - the Find label;
  - ⌘]/⌘[ scoped to the view on screen.
- Window titles.
- Copy: "Main checkout" (`SidebarViews.swift:208`); "project" goes (`Shortcuts.swift:115`, `QuickCreate.swift:160`,
  `ContentView.swift:2693`).
- Tests:
  - `MT/ShortcutSheetTests.swift`:
    - `@Test("No two menu bar items share a chord")`: new; it would have caught ⇧⌘↩ and ⌘[
    - `@Test("No shortcut copy says project")`
  - `MT/PaletteWorkspaceTests.swift`:
    - `@Test("Typing a task key finds the task")`
    - `@Test("A workspace with no matching worktree is still found")`

### 2F. First-launch tip

**2F.1** `tips.workspaces`, shown once.
- Test: `@Test("The tip shows once")`.

**Slice 2 is done when**, on a full-screen 13" Mac, a workspace shows its orchestrator and board, a task opens its
agent and changes, and Needs You answers an ask. Check this by driving the real app, leaving the live app's
process alone.

---

## Slice 4: the phones (→ 1C, 1D, 1E, 3A)

### 4A. iOS (one lane)

**4A.1 The root and the store.**
- `ios/FleetStore.swift`: a per-runner needs-you store. It reads `needs_you` through the FFI, re-reads on the
  `needs_you` event line, and falls back to `NeedsYou.derived` on an older runner. This is the store 4C reads.
- `ios/FarCoolerApp.swift`: a `NavigationStack`.
- New `ios/NeedsYouScreen.swift`: the items; the Workspaces sections; Unclaimed and Hidden per repository, below
  its workspaces; and the not-answering caveat.
- Launch follows ruling 4.
- Tests (`AKT/ShellNavigationTests.swift`):
  - `@Test("Launch lands on Needs You when it has items")`
  - `@Test("With nothing waiting, launch pushes the last workspace over Needs You")`

**4A.2 The workspace screen.**
- New `ios/WorkspaceScreen.swift`: a segmented Orchestrator, Board and Worktrees control, remembered per workspace.
- The board is in-line, from `sections`: every status, with `0` headers collapsed. Retire `BoardSheetHost`
  (`ShellScreen.swift:2272`) and `landOnBoardJump` (`:2259`).
- Start Orchestrator, Restart and Replace, through 1C.3; the 30-second state.
- New Worktree claims for the workspace.
- UI tests (`apps/ios/FarCoolerUITests/WorkspaceScreenTests.swift`), run only through `scripts/ios-ui-tests.sh`:
  - `testWorkspaceShowsOrchestratorBoardAndWorktrees`
  - `testStartOrchestratorFromAnEmptyWorkspace`
  - `testAnEmptyStatusIsACollapsedZeroHeader`

  These replace `ShellBoardTests.swift`.

**4A.3 Task and worktree screens.**
- New `ios/TaskScreen.swift`: the card with Answer buttons, through 1C.3's `task.note`; then Agent, Changes and
  Worktree rows that push.
- The worktree screen is today's shell, scoped to one worktree:
  - the overview retires;
  - "Diff" becomes "Changes" (`ShellScreen.swift:294`);
  - the crossing note (`:1823`) retires;
  - `ShellFleetMap.resume`'s remembered terminal carries over;
  - the pane bar shows the task chip via `TaskLink`.
- Deep links push the workspace, then the task, then the agent.
- UI tests:
  - `testGoingToATasksAgentAndBackReturnsToTheTask`
  - `testANotificationTapLandsWithTheWorkspaceUnderIt`
  - `testAnsweringADecisionRemovesItFromNeedsYou`

**4A.4 Vocabulary.**
- "Quick Task" becomes "New Worktree…", with the `quicktask.*` drafts migrated.
- "Project" becomes "Repository".
- The byline becomes "Orchestrator" (`AK/TaskBoardModel.swift:731`).
- VoiceOver loses "pane", "tab" and "session".
- Tests:
  - `AKT/TaskBoardModelTests.swift`: `@Test("An orchestrator's note is bylined Orchestrator")`
  - `AKT/ShellIdentityTests.swift`: `@Test("No VoiceOver label says pane, tab or session")`, a source scrape over
    `ios/`, which fails on a planted string

### 4B. Android (one lane, ∥ 4A)

**4B.1 Routes.**
- `android/ui/Navigation.kt`:
  - `Workspace(host, workspace, tab)`;
  - `Board` stays as a deprecated route that is mapped to `Workspace(tab = board)` on decode (kotlinx.serialization
    has no alias);
  - `decodeStack` drops only an unknown `Fleet` entry, instead of the whole stack (`:323-328`).
- Tests (`androidT/ui/BackstackTest.kt`):
  - `` `an old saved Board route restores as the workspace's board tab` ``
  - `` `an old saved Fleet entry is dropped and the rest of the stack restores` ``
  - `` `back from a task's agent returns to the task` ``

**4B.2 Needs You.**
- `android/model/NeedsYou.kt` and `ui/NeedsYouScreen.kt`:
  - merged items, labeled by workspace, from the FFI store with the `derived` fallback;
  - delete the worktree sections and the Board band (`NeedsYouScreen.kt:280-291`);
  - the Workspaces list, with Unclaimed and Hidden.
- The drawer holds the Workspaces list.
- Tests (`androidT/model/NeedsYouTest.kt`, rewritten):
  - `` `items are labeled by workspace and ordered by rank` ``
  - `` `nothing needs you is never shown above a decision` ``
  - `` `a finished agent is not an item` ``

**4B.3 Workspace, task and worktree screens.**
- New `ui/WorkspaceScreen.kt`: a `TabRow`.
- The Board tab uses `sections`, with collapsed `0` headers.
- The task screen gains Changes and Worktree rows (the first reader of `TaskRow.worktreeId`) and Answer, through
  `task.note`.
- The worktree screen gets a back arrow when it's pushed.
- Start Orchestrator; New Worktree claims for the workspace.
- Vocabulary: `FleetScreen.kt:332`, `BoardScreen.kt:116`, `NeedsYouScreen.kt:404`.
- Tests:
  - `` `a task with a worktree but no agent reaches its changes` ``
  - `` `an empty status is a collapsed zero header` ``
  - `` `the board tab's title is Board` ``

### 4C. Glances (one lane, → 4A.1 and 1B; owns the widget, Live Activity and watch files)

**4C.1 Snapshot.**
- `AK/FleetSnapshot.swift` gains optional `needsYou: [NeedsYouItem]`.
- `ios/FleetSnapshotWriter.swift` writes it from 4A.1's store.
- Tests (`AKT/FleetSnapshotTests.swift`):
  - `@Test("A snapshot without needsYou decodes as before")`
  - `@Test("The widget's count is the item count")`

**4C.2 The watch's data path.**
- `ios/WatchLinkHost.swift`'s `send(snapshot:)` (`:215-255`) already carries `FleetSnapshot` in the application
  context. The watch decodes `needsYou` there.
- `FarCoolerWatch/FleetListView.swift`: a Needs You section first. Asks answer through the existing
  `answerFromGlance` path; decisions and reviews show "Open on iPhone".
- `WatchFleetWidget` shows the count.
- Tests:
  - new `AKT/WatchStateTests.swift`: `@Test("The watch lists items before agents")`
  - `@Test("An ask's buttons on the watch are the item's actions")`

**4C.3 Live Activity.**
- The header uses the relay's `needsYou`.
- Rows are relabeled "Billing · claude".
- The ov-54 Allow and Deny buttons survive.
- Tests (`AKT/AgentCardRowsTests.swift`):
  - `@Test("The header counts needsYou when the relay sends it, else blocked")`
  - `@Test("A row names its workspace")`

### 4D. Delete `listed` (→ 4A, 4B)

**4D.1** Delete `TaskBoardModel.listed` in AgentKit and Kotlin. The build is the test: nothing reads it.

**Slice 4 is done when** both phones open on Needs You, answer an ask and a decision, and go workspace → task →
agent → back to the task, and the lock screen's count matches the app's after a decision is answered.
