# ov-1397 · Android and web removal audit

Audit base `705e8767c`. Scope is `apps/android`, `crates/android`, `services/web`, and `crates/web`.

## What these codebases must do

The Main workspace charter’s Nov 9 goals are to make the orchestrator configure, plan, feed and supervise work through the workflow engine, and to reduce the work of shipping a change by implementing feature logic once in shared Rust under R-112. The phone jobs come from [Jobs to Be Done](../../docs/jobs-to-be-done.md#phone). The browser is outside that document's surface map, so its job assignments below are inferred from the charter and the routes the browser actually exposes.

| Requirement | What to preserve |
| --- | --- |
| Charter goal 1 and the [workflow engine contract](../../docs/engine-pipeline.md) | Follow workspace plans, boards, tasks, runs, and reasoned decisions. Read engine state and send explicitly chosen actions to the authorized runner. Intake and work progression belong to the engine. The owner retains plain terminals for their own scripts. |
| Charter goal 2 and R-112 | Render shared client presenters and daemon-derived state. Keep the Kotlin/JNI and TypeScript/Wasm boundary, platform security, input, navigation, rendering and lifecycle code where callers need them. A live client twin requires a migration with parity evidence before removing its implementation. |
| Phone Reassure and Unblock, web inbox | Open on fleet attention, report stale or disconnected state, show agent-provided choices, and deliver live answers to the correct runner and task. Keep notification and deep-link entry points. |
| Phone Capture and Decide, web task and agent context | Start or capture work into the workspace workflow, return to the relevant task or conversation, read the context and apply judgment. Preserve interrupted drafts, workspace and runner identity, and resumable detail. |
| Phone Review | Read diffs, navigate files and tasks and resume review after interruptions. Keep controls that deliver review via the current task or agent path until an equivalent path is available. |
| Phone Tend for pairing and web access | Securely join, connect and reconnect, and manage the credentials required to do so. Preserve grant checks, sign-in, recovery, and protocol compatibility where existing callers depend on them. Broader Android administration needs its own product decision. |

## Inventory and reachability

The inventory starts with `git ls-files apps/android crates/android services/web crates/web`. It includes build and test files, generated wire projections and checked-in assets. It excludes local build outputs and installed dependencies. I indexed declarations and references across the source trees, then followed callers and searched the wider repository for proposed removals, including exact JNI/export names and their Kotlin/TypeScript spellings. A lexical hit in documentation, an import, a test or a bridge declaration alone does not establish a production caller.

| Area | Tracked files | Entry points and disposition |
| --- | ---: | --- |
| `apps/android` | 570 | Manifest application, activity, links, notification service and receiver. `MainActivity` enters `RootScreen`. `AppModel` and `DestinationDriver` handle navigation, restored state and external arrivals. `ui` renders the phone jobs, `net` owns connection and read lifecycle, `account` / `ceremony` / `data` support identity, enrollment and saved choices, `notify` supplies glance and answer paths, and `core` bridges to Rust. The model inventory below traces the ratcheted twins. Debug activities, instrumented tests, JVM tests, resources and Gradle wiring were included in reference searches. |
| `crates/android` | 3 | Cargo manifest, JNI entry points in `src/lib.rs`, and shared workflow presentation in `src/workflow.rs`. Checked Kotlin externals, Gradle packaging and R8 rules as well as Rust references. |
| `services/web` | 116 | The Worker in `src/index.ts` dispatches version, hashed assets, CSP reports, auth, sign-out and the admitted relay API. Its HTML loads the bundle from `scripts/build.mjs`'s `src/client/app.ts` entry. Client modules cover setup/recovery, fleet/inbox, workspace Plan and task detail, Workflow Runs, conversation and terminal. Worker and browser test suites, styles and build configuration have distinct entry points. |
| `crates/web` | 31 | Wasm bindings in `src/js.rs`, SSH/tunnel session and liveness, wire/plan and workflow presenters, enrollment/passkey/registration, and agent rows/input. The native tests and examples also consume this non-published crate. |
| **Total** | **720** | The counts include tests and supporting files. |

Android navigation traced from the manifest and `ui/MainActivity.kt` through `ui/RootScreen.kt`, `ui/AppModel.kt`, `ui/Navigation.kt` and `ui/DestinationRoutes.kt`:

- No runners opens Onboarding/Authorize/Join. With runners, Needs You and its drawer open workspace rows and unclaimed/hidden worktrees. Workspace rows open `WorkspaceScreen` with Orchestrator, Board and Worktrees/Themes, plus the wide Plan/tree layouts. The former standalone Board discriminator decodes to the Workspace Board tab. Stored `fleet` entries are skipped during backstack decoding. Those restoration paths have existing callers.
- Board, Plan and tree actions open task detail, history, plan pages, nested tree levels and panes. The wide layout and the compact Themes tab both reach `TreeRoot` and then `TreeLevelScreen` / `SidebarListScreen`. A selected worktree can open Changes, Files, native conversation and terminal views. Changes opens the index, history, base and review composer/outbox sheets. `WorkflowRunsView` is reached from the workspace toolbar, and Needs You opens workflow change review.
- Settings is reachable from the drawer and opens runner settings, device management and address/enrollment flows. Notifications and inbound destinations can open task/terminal roots directly. These routes and the debug-only Glance activity explain why several names with sparse references still have callers.

Web navigation traced from `src/index.ts`, `src/client/index.html` and `src/client/app.ts`:

- Worker admission and sign-in lead to browser enrollment or saved-copy restore, then the runner picker. A successful connection displays Terminals and wires Needs You, Plan, task detail, the workspace picker and Workflow Runs. Selected agents open with conversation and have a Terminal toggle. The Workflow Runs dialog uses `WorkflowSessionPort`. The Plan reads engine runs and landing groups through `planView`. Existing task notes, questions and decisions use their own live RPC paths.
- There is a live Saved Copies entry in addition to sign-out and the native enrollment and recovery paths. `WEB_CLIENT` is deliberately closed by default and set to `everyone` for canary in `wrangler.toml`. `flag.ts` also consumes the owner allowlist. I found no unset flag that established a removal. Retired web paths below are unused branches or exports of the browser bundle and its Wasm module.

## Ranked removal tickets

Counts are **physical production source lines** in the listed inclusive ranges, including comments. A whole file is identified as such. Companion tests and incidental import/comment cleanup are listed or described separately and excluded from both totals. For owner decisions the total is the gross footprint of the listed code, before any replacement UI or wiring.

Path prefixes in the tables expand as follows.

| Prefix | Directory |
| --- | --- |
| `A/` | `apps/android/app/src/main/java/com/farcooler/` |
| `AT/` | `apps/android/app/src/test/java/com/farcooler/` |
| `W/` | `services/web/src/client/` |
| `R/` | `crates/web/src/` |
| `J/` | `crates/android/src/` |

### A. Safe removal now

| Rank and proposed ticket | Paths and lines | Evidence and boundary |
| --- | --- | --- |
| A1 · Remove the pre-workspace Android front door remnants | `A/model/TaskBoard.kt:278–279,443–491,539–585,610–629` (118), `A/model/FleetLayout.kt:83–99` (17), `A/ui/FleetScreen.kt:822–866` (45), `A/model/Model.kt:1085–1087` (3). **183 lines.** | `rg -n 'WorkspaceHeading\|FleetLayout\.(orchestrators\|boardRows)\|RunnerBoards\.rows\|BoardRow\|tasksWithLiveAgents\|landingTerminal' apps/android/app/src` finds the heading definition, test-only row builders, stale imports, and the fleet overload tested only by `ModelTest`. The actual front door is `NeedsYouScreen` → `WorkspacesList.WorkspaceListRow` → `Route.Workspace`. The worktree tab uses `TreeRoot` or the scoped `WorktreeList`. Keep live `RunnerBoards.boards/touched/decisions/waiting`, `TaskAgentLink`, `FleetLayout.orchestratorTitles` and the **list** `landingTerminal` used by `Backstack.rule`. Update the old-row cases in `AT/model/TaskBoardTest.kt` and `AT/model/FleetLayoutTest.kt`, and retarget `AT/model/TaskLinkTest.kt` and `AT/model/ModelTest.kt` to live pane behavior. |
| A2 · Remove the replaced Android chat helpers | `A/model/Composer.kt` (47, whole), `A/model/PermissionAnswering.kt` (85, whole), `A/model/PromptImageBudget.kt` (87, whole), `A/ui/PromptImageEncoder.kt` (56, whole). **275 lines.** | Commit `0e7ac3154` removed the old pane's call sites. `activeToken` / `ComposerToken` and `PermissionAnswering` now have only self and test references. `promptImageEncoder` has no caller, and the only other `PromptImageBudget` consumer is its test. The live composer is `NativeAgentView` with `NativePane` and `OutgoingImageEncoder.convertForRunner`. The permission UI calls `NativeAskRow` → `NativePane.answer` or the separate Needs You answer. Remove `AT/model/ComposerTest.kt` and `AT/model/PromptImageBudgetTest.kt` with their orphaned implementations. Move the `not-yours` expectation in `AT/model/RunnerRefusalTest.kt` to the live answer path or verify existing coverage before dropping it. The abandoned completion tokenizer has no equivalent currently in this Android composer. |
| A3 · Remove the web reply-accept export for the retired unsealed enrollment leg | `R/js.rs:210–222` (13), `R/enroll.rs:197–219` (23), `W/wasm.d.ts:30–32` (3). **39 lines.** | Repository-wide `rg -n 'acceptReply\|enroll::accept'` finds only the export, the declaration and its delegate. Direct `accept` calls in `R/enroll.rs` are unit tests. `crates/web` sets `publish = false`. `W/app.ts` creates an offer with a reply secret and calls `openReply` → `enroll::open_all` (ov-1020 and ov-778), then compares and keeps the sealed reply. Preserve freshness/replay coverage for that live path and the native ceremony implementations. Replace obsolete `accept` cases in `R/enroll.rs` only where the sealed suite needs coverage. |
| A4 · Remove the unused browser orchestrator-page facade and type aliases | `R/pages.rs` (15, whole), `R/lib.rs:23` (1), `R/js.rs:381–385` (5), `W/wasm.d.ts:81` (1), `W/workflow.ts:7–8` (2). **24 lines.** | `rg -n '\.page\(\|WorkflowStage\|WorkflowForm' services/web crates/web` finds `WebRunner.page` only delegating to `Runner.page` and the shared session, with no browser caller. Chat's same-name `page` takes row sequence and size on a different receiver. Plan reaches its live task read via `openCard` and Workflow Runs reads workflow presentation. The two aliased types have only declarations. Trim their unused imports on line 5. Keep generated types, shared page support and Android Plan Page routes. |
| A5 · Remove the leftover web recovery preflight wrapper | `W/vault.ts:189–194` (6). **6 lines.** | `rg -n 'relayHoldsOurs\|syncCopy' services/web` finds `relayHoldsOurs` calls only in `services/web/test/vault.test.ts`. The live `app.ts:177–190` uses `syncCopy` to read, merge and handle conflicting or incomplete copies. Remove the isolated preflight case and its import, and keep the restore case's existing assertions about `needsSaving`, credential and runners. Keep the authenticated recovery read and merge logic. |
| A6 · Remove the Android legacy push-pairing request and unused terminal JNI hooks | `A/account/Account.kt:284–298` (15), `A/core/Native.kt:311,328` (2), `J/lib.rs:981–988,1110–1117` (16). **33 lines.** | Repository-wide `rg -n 'pairDaemon\|nativeRevision\|nativeTakeBell'` turns up the Android definitions, the paired Rust symbols and an Android R8 comment. The same-named relay handler and Swift helper have different receivers. `services/relay/src/directory/runners.ts` names directory runner registration as the successor to `/v1/daemons`. Keep Android push registration and existing revoke calls. Neither JNI hook is called in any Android source set. `VtCore.snapshot` is live. Update the explanatory R8 comment and keep the shared VT C ABI. |
| A7 · Remove declaration-only Android UI and state helpers | `A/net/TerminalSession.kt:287–298` (12), `A/model/Shell.kt:713–718` (6), `A/model/ReviewComments.kt:276–278` (3), `A/ui/TaskKeyCardDialog.kt:12–13,59–61` (5). **26 lines.** | `rg -n 'reassertSize\|columnProgress\|clearFailure\|taskKeyDescription' apps/android/app/src` finds only declarations. The terminal session still calls `scheduleResize` directly on active size and lifecycle paths. `PaneTrack` uses live Shell geometry. Review queue submission and the task key card remain reachable. |

### B. Working surfaces for the owner to decide

| Rank and owner question | Paths and lines | Current path, alternative and what would be lost |
| --- | --- | --- |
| B1 · Can runner theme authoring move off Android? | `A/ui/RunnerEditorScreens.kt` (306, whole), `A/ui/RunnerSettingsScreen.kt:96,101–112,162,195,202–220,393–435,761–777` (94), `A/net/Connection.kt:1777–1797` (21). **421 lines.** | Settings → Runner settings → Themes calls `ThemeEditorScreen`, `theme.upsert` and `theme.delete`. The editor includes a local preview. The CLI already sends the same theme write RPCs (`crates/cli/src/main.rs:1642,1678`). The phone jobs limit Tend to pairing. Removing authoring loses creating, duplicating, previewing, editing and deleting runner themes on a phone. Preserve reading themes and choosing existing themes. |
| B2 · Can Android retire launching agent presets from New Terminal while retaining a plain shell? | `A/model/QuickTask.kt:100–120,136–138` (24), `A/ui/Sheets.kt:647–648,652,669–687,697–699` (25). **49 lines.** | `FleetScreen` and the Themes tree in `PhoneTree` call `NewTerminalSheet`, which offers Claude, Codex and Cursor and passes the selected model to `Connection.createTerminal` (`terminal.create`). The workspace orchestrator and engine enqueue/track/agent stations are the paths for Far Cooler work (charter goal 1). The owner loses ad hoc agent and model selection inside an existing worktree. Keep shell launch, existing sessions, orchestrator start/replace and worktree recovery. This is a gross count of the preset/picker blocks to remove or replace, with a small shell-only call-site rewrite needed. |
| B3 · Should the web open on Needs You and require a deliberate terminal selection? | `W/app.ts:846,950–952,958` (5), `W/scope.ts:70–74` (5). **10 lines.** | `connectTo` calls `showTab('terminals')`. On initial load and a workspace switch, `relist` calls `firstToOpen` then opens the first running pane or the first listed pane. Needs You, Plan and explicit pane buttons already work. Android's Needs You front door is the precedent. This loses instant access to whichever pane the current ordering picks, including on a workspace change. Preserve explicit conversation and shell access and replace the clear/empty-state behavior as needed when the current pane falls out of scope. `services/web/test/scope.test.ts` pins the existing selection rule. |

### Totals

- **A · 586 production lines** across seven proposed removal tickets. The numbers include the two dead twins in the baseline.
- **B · 480 production lines** across three owner decisions, gross before replacement wiring. These are outside the A total. Test/fixture lines and uncertain findings are outside both totals.

## Android model twins

`jq -r '.twin[]' scripts/shared-core-baseline.json` lists 59 paths. The 58 Android models are below, with two dead files and 56 files with live code. The other entry, `W/workflow-session.ts`, is live through `app.ts` → `WorkflowSessionPort` and handles authenticated runner session and action intent across reconnects. The twin rule matches file names and port headers, so the presence of a baseline entry alone does not prove that a current Rust equivalent exists.

All filenames below live under `A/model/`. Counts cover the entire file and are inventory only. A live anchor establishes at least one use of the file, with exact dead ranges reported under A where found.

| Model | Lines | Disposition and production anchor |
| --- | ---: | --- |
| AgentConversation.kt | 693 | Live. `NativePane` and `NativeAgentView` use conversation state and word handling. |
| AgentRowLedger.kt | 189 | Live. `AgentRowStore` applies follow and pagination updates. |
| AgentRowWire.kt | 531 | Live. `AgentRowStore` decodes pages and changes consumed by native conversation. |
| AskAboutTask.kt | 160 | Live. `BoardScreen` and `AskOrchestratorItem` prepare task context and route it to the orchestrator. |
| BoardReadsKeeper.kt | 161 | Live. `BoardReadsKeepers` tracks per-board read position. |
| BoardReadsSync.kt | 124 | Live. `BoardReadsKeeper` merges seen-through updates and `BoardReadsKeepers` sends them. |
| BoardSummary.kt | 180 | Live. `BoardUnread` renders summary and `Connection` uses task note decoding. |
| BringHere.kt | 146 | Live. `NativePane` uses it during conversation attachment and handoff. |
| Capability.kt | 92 | Live. `Model`, `BringHere` and `PrActions` read runner capability gates. |
| ClaudeSignIn.kt | 233 | Live. `ClaudeSignInSection` and `ClaudeSignInCalls` handle sign-in needed to unblock work. |
| Composer.kt | 47 | Dead, A2. `activeToken` is exercised only by `ComposerTest`. |
| Destination.kt | 222 | Live. `AppModel` and `DestinationDriver` handle saved locations and inbound destinations. |
| DestinationPayloads.kt | 156 | Mixed. `MainActivity.handleIntent` interprets Android arrival payloads. The string convenience parser `fromUrl` is uncertain below. |
| DestinationResolver.kt | 292 | Live. `DestinationDriver` resolves runner, workspace and pane arrivals. |
| DiffComputation.kt | 170 | Live. `ChangesScreen` and `DiffLayout` render review diffs. |
| EnrolledDevices.kt | 90 | Live. `EnrolledDevicesCalls` and `EnrolledDevicesSection` support credential management. |
| Files.kt | 476 | Live. `FilesScreen` uses its loader and directory/code view models. |
| FirstRun.kt | 162 | Live. `MainActivity`, `NeedsYouScreen` and `WorkspaceScreen` use first-run policy and copy. |
| GeneratedFiles.kt | 134 | Live. `ChangesState` and `Changes` classify generated diff files. |
| Glance.kt | 929 | Live. Glance UI, Fleet and terminal tabs consume marks, typography and palette. |
| HeldDraft.kt | 98 | Live. `DraftCalls` and `HeldDraftBar` decode and display queued drafts. |
| KeyRefusals.kt | 151 | Live. `Connection` and settings use key refusal and runner admission state. |
| LfsNotice.kt | 41 | Live. `ChangesLfsNotice` and worktree/Changes show the review notice. |
| LiteralOperation.kt | 64 | Live. `AgentRowWire` decodes it and `NativeToolRows` renders it. |
| LostPane.kt | 77 | Live. `Connection` and `TerminalPane` use lost-pane recovery information. |
| MemberOwnLogins.kt | 72 | Live. Runner Settings → Member Logins uses `MemberLoginsSection` and `MemberOwnLoginsCalls`. |
| NeedsYou.kt | 485 | Live. `Connection`, `NeedsYouScreen`, `WorkspacesList` and `FleetScreen` use answer routing, rows and worktree scope. Shared aggregation runs via `NeedsYouPresenter`. |
| NotLivePane.kt | 26 | Live. `TerminalPane` displays its inactive-pane state. |
| OneTree.kt | 395 | Live. `PhoneTree` renders workspace tree routes with `SidebarLists`. |
| OutgoingImage.kt | 51 | Live. `NativePane`, `PaneImageFiles` and `OutgoingImageEncoder` use it for outgoing attachments. |
| Page.kt | 495 | Live. `PageReads`, `PageLive` and `PageView` support Plan pages. |
| PageLive.kt | 374 | Live. `PageView`, `PlanPages` and `PlanPageScreens` use its links and layout. |
| PaneClaudeLogin.kt | 42 | Live. `Model` decodes it and `TerminalPane` shows the login notice. |
| PeopleAccess.kt | 313 | Live. `PeopleAccessController`, `PeopleAccessSection` and teammate ceremony code use roles and device grants. |
| PermissionAnswering.kt | 85 | Dead, A2. Only `RunnerRefusalTest` consumes the retired pane's state machine. |
| PhoneEmptyStates.kt | 88 | Live. `NeedsYouScreen`, `WorkspaceScreen` and `PhoneEmptyRows` show the phone's first-run and empty state. |
| PlainLine.kt | 31 | Live. `PlanStrip` uses it for the workspace header. |
| PlanAgents.kt | 83 | Live. `Plan` decodes agents, and `PlanSectionRows` / `PlanPageScreens` read their task or model. |
| PlanCost.kt | 297 | Live. `Plan`, `PlanCostViews` and Plan pages display spend context. |
| PlanGlanceSentence.kt | 69 | Live. `PlanGlanceBand` and `PlanScreens` use the shared view's glance sentence. |
| PlanLandings.kt | 233 | Live. `Plan` decodes data used by `PlanLandingCard`, `PlanSectionRows` and `PageLive`. |
| PlanPrStage.kt | 98 | Live. `PlanTicketStage` renders PR stage and `PrActions` / `NeedsYou` use links. |
| PlanRulings.kt | 162 | Live. `PlanRulingRows` renders rulings and `RulingActions` acts on them. |
| PlanSidebarTrees.kt | 170 | Live. `SidebarScreens` builds theme/ticket models for the workspace tree. |
| PlanStrip.kt | 138 | Live. `PhoneTree` and `WorkspaceCards` summarize orchestrator and Plan status. |
| ReviewComments.kt | 411 | Mixed. `ChangesStore`, `ChangesSheets` and `ReviewStorage` use review anchors, notes and receipts. Only `clearFailure` is in A7. |
| RunnerRefusal.kt | 231 | Live. `ClientCore`, `ActionNotices`, `TerminalSession` and Changes show refusals. |
| RunningAgents.kt | 245 | Live. `WorkspaceScreen` and `WorkspaceCards` display live agents. |
| Shell.kt | 1083 | Mixed. `PaneTrack` and `WorktreeScreen` need pane gestures and geometry. Only `columnProgress` is in A7. |
| ShellClose.kt | 121 | Live. `TerminalTabStrip` and `WorktreeScreen` use pane-close behavior. |
| TaskBoard.kt | 911 | Mixed. `Connection`, `BoardScreen` and `BoardReads` use board state, grouping, and read/sweep policy. The old front door row code is in A1. |
| TaskKeyCards.kt | 77 | Live. `TaskKeyLinking` and `TaskKeyCardDialog` supply task context. |
| TaskLink.kt | 127 | Live. `Notifier` and `AppModel` route push arrivals and `TerminalPane` links the task. |
| TaskStartLines.kt | 306 | Live. `TaskBoard` decodes start/worker state and `BoardScreen` draws it. |
| TaskUsage.kt | 115 | Live. `Connection` reads usage and `TaskUsageSection` uses the client core presentation. Formatting helpers still have Plan/page callers. |
| WorkflowChangeReview.kt | 71 | Live. `NeedsYouScreen` opens the review on a workflow decision. |
| WorkspaceGroups.kt | 215 | Live. `Connection`, `BoardReads` and `NeedsYou` use workspace identity, grouping and notices. |
| WorktreeOrder.kt | 120 | Live. `FleetScreen` and `WorktreeActions` use worktree ordering. |

## Uncertain or retained

- **Compatibility lifetime is uncertain.** `A/ui/Navigation.kt:455–489` handles retired `fleet` and `board` encodings. Their screens have moved, and restore still calls the decoder. Android implicit workspaces and web's unscoped terminal list remain reachable on older runners. Removal needs the supported saved-state and runner version policy.
- **Android sealed bridge coverage is intentional today.** `A/ceremony/CeremonyStore.kt:531–532` delegates to the JNI sealed-reply endpoint. The only direct caller is `apps/android/app/src/androidTest/java/com/farcooler/ceremony/SealedReplyDeviceTest.kt:65`. Its removal is uncertain until the device test covers the production signed-reply path at an equivalent boundary. The browser removal in A3 is for the unsealed reply path.
- **Some test-only helpers still assert production contracts.** `A/model/DestinationPayloads.kt:115` (`fromUrl`) is called by `DestinationTest` against shared fixtures. `A/model/ChangesState.kt:302` (`isUntracked`) is called by `ChangesStateTest`. `A/net/NativePane.kt:412,597,757` provides draft and attachment test seams. These are uncertain pending confirmation that fixtures can be expressed through real Android intents, review selectors and draft lifecycle calls.
- **Manual worktree/recovery and review are distinguishable from launching work.** `NewWorktreeSheet`, `ResumeBranchSheet`, and the Changes outbox have live paths. It is uncertain which owner repair, investigation and resumable review scenarios would survive moving those capabilities to the engine. B2 counts only the hosted agent-launch choices. Existing worktree changes and shell behavior are outside that estimate.
- **A shared tree does not automatically replace a live detail view.** Web Plan notes/rulings and discussion paths still carry context beyond engine control (`W/plan-tab.ts`, `W/plan-view.ts`, `W/detail.ts`). Android device access and runner sign-in have live pairing and Unblock dependencies even though broader Tend is open. The amount removable through additional Rust migrations is uncertain until callers and behavior have a verified replacement.

## Verification

- Ran `python3 scripts/shared-core-lint.py --self-test` and `python3 scripts/shared-core-lint.py`. Both passed. The current lint reported no new entries and five baseline entries eligible to shrink. The per-file audit uses the committed baseline.
- Ran `python3 scripts/plan-layer-lint.py`, `python3 scripts/page-layer-lint.py`, and `python3 scripts/web-client-tree-check.py --self-test`. All passed.
- Ran `tsc -p tsconfig.client.json` and `npm run test:workflow` against a temporary copy of the audited web tree with repository fixtures and cached dependencies. Both passed. The workflow script uses fixture/controller and simulated DOM tests.
- Ran the vault, sealed enrollment, Plan and Plan presenter Vitest suites under Node in the temporary web copy. All 55 tests passed. The default workerd pool stopped before collecting tests in this sandbox.
- Ran `python3 scripts/ci-changes.py --self-test`. It passed, including the report-only path classification.
- This report-only commit has no product runtime test delta. The reachability conclusions are source traces, with no device or deployed-browser traversal. Full Gradle, Cargo and Wasm builds were excluded by the ticket.
