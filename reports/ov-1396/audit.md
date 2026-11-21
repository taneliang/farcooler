# ov-1396 Apple app prune audit

Source snapshot: `705e8767c`. Requirements come from the Main workspace charter
(`d7401571`, Goals, owner Nov 9), [Jobs to Be Done](../../docs/jobs-to-be-done.md)
and the [workflow engine design](../../docs/engine-pipeline.md). The charter
puts the orchestrator in charge of configuring the process, planning, feeding
work through the engine and supervising execution (goal 1). Plain terminals
remain available for the owner's scripts. Feature logic belongs once in shared
Rust under R-112, with thin app renderers to reduce the cost of shipping a
change (goal 2).

## What Apple needs to do

| Surface | Requirement | Reachable implementation to preserve |
| --- | --- | --- |
| Mac console | Reassure and Unblock across the fleet. Navigate quickly to the place needing attention and back. | `FarCoolerApp` → `ContentView`; sidebar, Needs You, Next Needing Attention, and ⌘P. |
| Mac cockpit | Capture, Decide, Drive and Review at the desk. Inspect a patch, talk to or steer an agent, and use a plain terminal for personal scripts. Goal 1 puts work intake, execution, checks and landing under the engine. | `ContentView+WorkspaceDetail` and `ContentView+Detail` → workspace Plan, task, conversation, worktree, changes and workflow runs. |
| Mac setup | Tend runners, access, workspaces and the rules the orchestrator follows. | `FarCoolerApp` Settings scene and `ContentView` runner, repository, workspace and charter sheets. |
| Phone and iPad | Reassure, Unblock, Capture, Decide and Review away from the desk. Pair the phone, carry work across interruptions, and resume review in short windows. | `FarCoolerApp` → `PhoneRoot` → Needs You, workspace, task and Plan routes, Agents sheet and Worktree cover. Links, notifications and restoration also feed the navigator. |
| Live Activity | Reassure and Unblock for one run on the locked phone. | `FarCoolerActivityBundle` → `AgentActivityWidget` and `AnswerPermissionIntent`. |
| Phone widgets | Reassure for the fleet from home or lock screen. Keep the cached data current when the phone app is absent. | `FarCoolerActivityBundle` → fleet widgets; the notification service extension and shared snapshot store. |
| Watch app | Reassure, Unblock and Capture in short or offline moments. Keep Decide one tap deeper and prefetch its context. | `FarCoolerWatchApp` → `FleetListView` → `WatchRoute` destinations, Compose, and the phone relay through `WatchLinkClient`. |
| Watch complications | Reassure from the watch face using cached data. | `WatchFleetWidget` widget bundle and shared snapshot store. |
| Shared code and bridges | Goal 2 and the cross-surface laws. Render the same authoritative state, preserve freshness and capability checks, use supplied permission choices, and queue captures while answers require a live link. Keep native UI, lifecycle, persistence, accessibility and OS integration where the platforms need them. | AgentKit, Rust C ABI adapters, phone relay, and per-target App Group plumbing. Production entry points, platform callbacks and tests are consumers. |

## Inventory and method

The inventory covers 1,429 tracked paths, including 1,378 Swift files. An
identifier census covered 697 Swift implementation files and searched across
the test and fixture sources as well. Counts below are tracked files, including
non-Swift assets and build metadata.

| Area | Files | What exists and where it connects |
| --- | ---: | --- |
| `apps/macos` | 693 | 302 source and C module files, 378 test and support files, 13 packaging, settings, tool and build files. SwiftPM collects `Sources/FarCooler`; the source sweep covered the sidebar, command palette, menus, Plan, terminal and changes panes, Files, native agent views, enrollment and settings. |
| `apps/ios/FarCooler` | 156 | 150 Swift files plus resources and signing metadata. Source groups cover pairing, runner connection, notifications, Needs You, workspace, the phone stack and iPad columns, board and Plan, worktrees, Changes and Files, native conversation and preferences. |
| `apps/ios/FarCoolerActivity` and `FarCoolerNotify` | 9 | The phone widget bundle registers Fleet, Plan and Live Activity. The notification service is registered by its extension plist. Both consume shared snapshot or push data. |
| `apps/ios/FarCoolerWatch` and `FarCoolerWatchWidgets` | 15 | Nine watch Swift files and two complication Swift files, plus metadata. The Fleet screen opens agent and Needs You item routes; agent detail produces the compose, permission and transcript routes. The widget bundle registers Fleet and Plan independently. |
| Other `apps/ios` | 80 | 65 UI test files, eight NativeRowsFixture paths, four WorkflowFixture paths, the shared Xcode scheme, the project generator and export options. |
| `apps/shared` | 476 | 231 AgentKit source files, 239 test and resource files, three package/support files and three asset catalog files. SwiftPM exposes AgentKit to Mac; `apps/ios/generate-project.py` explicitly lists phone, watch, extension and test inputs. |

I enumerated the tracked trees with `git ls-files`, checked Apple target
membership, and screened declarations for no references or references confined
to their declaration file or tests. For the ranked findings I checked whole-word
`git grep` hits, callback and target registration, and navigation producers
alongside destination switches. I also searched settings keys, launch arguments
and feature flags for writers, and checked the app-to-Rust projections for
remaining Swift readers. The declaration scan is a lead generator. It can miss
indirect calls, overloaded names and dead groups that refer to one another.
The ranked counts are verified slices from this snapshot, with uncertain
candidates excluded.

## A. Dead code ready for removal

Ranked by gross lines removed. Each numbered item is one proposed removal
ticket. Line ranges are inclusive. For each ticket, drop any memberships for
deleted files from the iOS generator, update references in tests and lints, and
retain coverage for code with a live caller. Totals count the listed Swift
source and test slices, including comments and blank lines. Generator and lint
bookkeeping, replacement lines and unverified follow-on savings are excluded.

### A1. Remove the detached iOS composer accessory and its resize channel — 470 lines

`DockedBar` has no constructor call; its only external source hits are old
comments. `DockedBarController` is made only by that representable, and its
`AccessoryHostView` is the only production caller of
`AccessoryCoverChannel.post`. The remaining subscriber is the shell's
`KeyboardInset()`, which still reads UIKit keyboard frames for
`ShellScreen.paneBottom`. Keep those frame observers and cover tests. The live
conversation renders `NativeComposer` from `NativeAgentView`, and the terminal
has its own accessory in `TerminalView`. The removal includes the stale
accessory-specific observer and resend calls, plus tests for that channel.
Evidence searches: `git grep -n -w -E 'DockedBar|DockedBarController|AccessoryHostView|KeyboardInset|AccessoryCoverChannel|AccessoryScope'` and the exact notification strings
`FarCooler.AccessoryHostView.didResize` and `FarCooler.AccessoryHostView.reportAgain`.

| Path and lines | Lines |
| --- | ---: |
| `apps/ios/FarCooler/DockedBar.swift` 4–65, 89–95, 100, 115–118, 146, 150–162, 179–183, 186–203, 206–439 | 345 |
| `apps/shared/AgentKit/Sources/AgentKit/KeyboardCover.swift` 52–63, 66–100 | 47 |
| `apps/shared/AgentKit/Tests/AgentKitTests/KeyboardCoverTests.swift` 25–44, 63–84, 87–122 | 78 |

When applying this ticket, simplify the initializer and refresh keyboard
comments. Preserve the assertions for UIKit frame, hide and synthetic
full-screen frame behavior. The terminal and conversation keyboard UI tests
are the follow-up verification.

### A2. Finish removing the Mac typed-description task launcher — 415 lines

ov-951 removed `QuickCreate` and `DaemonClient.startTask` in `a0a9ce489`; the
phone equivalent was removed in `9f025236a`. Goal 1's intake route is the
workspace orchestrator, which files and feeds tickets into the engine. Search
`TaskName|TaskNamer|OnDeviceNamer|TaskPrompt|takesPrompt|defaultAgent` with
`git grep -n -w -E`. Hits for the naming pipeline are within the old
implementation and its tests. `TaskPrompt` and `takesPrompt` have no
executable caller. For the raw preference key `tasks.defaultAgent`, the
sole declaration and the Settings picker binding are the only production
accesses. The picker can change stored state but nothing that starts a worktree
reads it. The Mac's manual `NewWorktreeSheet` and ordinary shell command
have separate routes. Keep `TaskFailure` and its tests because live runner
refusals and message delivery use it.

| Path and lines | Lines |
| --- | ---: |
| `apps/macos/Sources/FarCooler/TaskName.swift` 2–247, 259–262 | 250 |
| `apps/macos/Tests/CeremonyTests/TaskNameTests.swift` 7–149 | 143 |
| `apps/macos/Sources/FarCooler/Preferences.swift` 71–76, 363–371 | 15 |
| `apps/macos/Sources/FarCooler/Composer.swift` 151–157 | 7 |

### A3. Remove three AgentKit helpers with test-only consumers — 357 lines

`git grep -n -w -E 'PaneFailureBanners|ShimmerAnimation|WorktreeOrder'` finds
only the declarations, their dedicated unit tests, project membership and
historical or lint references. Production use includes neither direct
construction nor an extension or selector using these names. Current failure
messages, animation and ordering still have live implementations. Preserve
the separate `DaemonBuild.keepsWorktreeOrder` capability and wire name in this
ticket so the removal is independent of compatibility policy.

| Path | Lines |
| --- | ---: |
| `apps/shared/AgentKit/Sources/AgentKit/PaneFailureBanners.swift` | 46 |
| `apps/shared/AgentKit/Tests/AgentKitTests/PaneFailureBannersTests.swift` | 33 |
| `apps/shared/AgentKit/Sources/AgentKit/ShimmerAnimation.swift` | 62 |
| `apps/shared/AgentKit/Tests/AgentKitTests/ShimmerAnimationTests.swift` | 75 |
| `apps/shared/AgentKit/Sources/AgentKit/WorktreeOrder.swift` | 68 |
| `apps/shared/AgentKit/Tests/AgentKitTests/WorktreeOrderTests.swift` | 73 |

### A4. Remove the unreachable phone branch-stack sheet — 168 lines

`git grep -n -w -E 'StackView|refreshPullRequests'` finds the iOS declaration,
the sheet's own refresh call, a lint allowlist entry, and Android references.
There is no iOS sheet, route, deep link or test instantiation. Real branch
adoption opens `BranchPicker` in `NewWorktreeView`. The current Changes
header still uses `ShellScreen.readPullRequest()` → `Connection.stack` →
`StackResponse`. Keep that read, its wire models/tests, and the picker.
The iOS `refreshPullRequests` allowlist row in `scripts/swallow-lint-allow.txt`
is dependent bookkeeping outside the counted Apple slices. The Android stack
and runner RPC remain live.

| Path and lines | Lines |
| --- | ---: |
| `apps/ios/FarCooler/BranchAndStack.swift` 153–314 | 162 |
| `apps/ios/FarCooler/Connection.swift` 1879–1884 | 6 |

### A5. Remove superseded phone attachment and pane helpers — 131 lines

`git grep -n -w -E 'PromptImageBudget|switchTo|RunnerSheet'` finds no call to any
of these Apple declarations. Native prompt attachments come from
`NativeComposeModel.attach` through AgentKit's `OutgoingImage.make`;
`ImagePasteQueue` handles plain terminal pastes separately. Pane changes in
`ShellScreen` and `ShellPaneTrack` switch between mounted views and keep a
`TerminalSession` per terminal, so `TerminalSession.switchTo` has no
retargeting caller. Runner settings open from `Settings` and `NeedsYouScreen`
with direct `RunnerSettingsView` construction; the `RunnerSheet` wrapper is
uninstantiated. Android hits for the first two names are separate code.

| Path and lines | Lines |
| --- | ---: |
| `apps/ios/FarCooler/ImagePaste.swift` 141–213 | 73 |
| `apps/ios/FarCooler/TerminalSession.swift` 442–491 | 50 |
| `apps/ios/FarCooler/ShellScreen.swift` 1291–1298 | 8 |

### A6. Remove the Swift Plan grouping left after the Rust presenter — 63 lines

`git grep -n -w -E 'PlanNowGroup|nowGroups|liveLandings|hasNow|PlanThemeGroups|themeGroups'`
finds the old grouping implementations and tests. The live phone and Mac
`PlanViewBridge` call Rust's `farcooler_client_plan_view`; the response
carries `landings`, engine `landingGroups` and `WireThemes`. Follow
`crates/client/src/plan_view/build.rs` and `landing_queue.rs` through to
`PlanSectionsView`, `PlanColumnView`, `PlanSectionRows` and
`PhonePlanColumn`. Their renderers use the wire projection. Keep Plan landing
decoding, historical rows, `lines(on:)`, CI lookup and unrelated tests.
The counted Mac test lines assert against the retired Swift grouping; keep
their rendered-card assertions. The AgentKit landing test also asserts
live helper behavior and can retain those assertions.

| Path and lines | Lines |
| --- | ---: |
| `apps/shared/AgentKit/Sources/AgentKit/PlanLandings.swift` 21, 175–181, 202–203, 218–237 | 30 |
| `apps/shared/AgentKit/Sources/AgentKit/PlanThemeTrack.swift` 37–51 | 15 |
| `apps/shared/AgentKit/Tests/AgentKitTests/PlanLandingsTests.swift` 45–47, 49 | 4 |
| `apps/shared/AgentKit/Tests/AgentKitTests/PlanThemeTrackTests.swift` 95–105 | 11 |
| `apps/macos/Tests/CeremonyTests/PlanLandingViewTests.swift` 59, 78–79 | 3 |

### A7. Remove unused Mac pane helpers — 61 lines

`git grep -n -w -E 'ToolLocationLabel|filesWorktree|moveToLayout|swapPanes'`
finds the label declaration and environment providers, with the label as
the only reader of `filesWorktree`. `OpenInFiles` itself has a live reader
in `ChangesPane`. The old layout helpers have no callers in app or tests.
Live drag-and-drop uses the edge-based `ContentView.placePane` path and
`DaemonClient.movePane`; the live Layout menu still uses `breakPane` and
other operations.

| Path and lines | Lines |
| --- | ---: |
| `apps/macos/Sources/FarCooler/Files/OpenInFiles.swift` 17–19, 22–50 | 32 |
| `apps/macos/Sources/FarCooler/ContentView+Panes.swift` 33, 169 | 2 |
| `apps/macos/Sources/FarCooler/ContentView+Tiling.swift` 176–197 | 22 |
| `apps/macos/Sources/FarCooler/DaemonClient.swift` 1762–1766 | 5 |

**Total for A · 1,665 lines** across seven candidate removal tickets.

## B. Working surfaces for the owner to decide

Ranked by the measured Swift source attributable to each surface. These are
conditional gross removal counts from the listed slices. Wiring in shared
views, fixture adjustments, generator updates and replacement work are
excluded. Each question names the existing job or capability that would go.

### B1. Should iPhone and iPad give manual checkout creation back to the desk? — 519 lines

This is reachable. The tree footer's `TreeWorktreeActions` makes the sheet
from `PlanSheetScreen`, `PhonePlanColumn` or `PadTreeColumn`. The
`newWorktreeSheets` modifier opens `NewWorktreeView`, and
`Connection.createWorktree` issues `worktree.create` for a shell. Users would
lose on-device creation of a named worktree and adoption of an existing
branch into a shell. The orchestrator and the engine cover intake and
execution for planned tasks. Picking up an outside branch or creating a
scratch checkout on mobile is a separate manual capability. Keep the
existing-worktree routes, review tools and repository registration entry
from Needs You. Follow-up would update the `new-worktree` UI coverage in
`ActionFailureTests.swift` and `WorkspaceScreenTests.swift`.

| Path and lines | Lines |
| --- | ---: |
| `apps/ios/FarCooler/FleetView.swift` 1014–1217 | 204 |
| `apps/ios/FarCooler/BranchAndStack.swift` 10–151 | 142 |
| `apps/ios/FarCooler/TaskSlug.swift` | 104 |
| `apps/ios/FarCooler/Connection.swift` 2000–2035 | 36 |
| `apps/ios/FarCooler/PadWorkspace.swift` 541–557 | 17 |
| `apps/ios/FarCooler/PhoneTreeScreens.swift` 114–129 | 16 |

### B2. Should the Mac retire direct agent launch from Resume Branch? — 344 lines

File → Resume Branch goes through `AppCommand.resumeBranch`,
`ContentView+Commands`, and the `ResumeBranch` sheet. It calls
`DaemonClient.adoptBranch`, which runs `worktree adopt` and launches an agent
with `terminal create --preset`. This is a working direct-launch path
alongside the orchestrator and workflow. Removing the whole counted surface
would lose branch discovery (including remote branches), adoption, choice of
agent preset and opening the resulting agent in one flow. Engine intake and
steering cover tracked work. Existing-branch adoption for personal or
off-board work needs an explicit decision before retiring this sheet.
Keeping a shell-only adoption UI would yield smaller savings. ov-951
deliberately retained the menu as a separate command, and
`TaskStartRemovedTests.swift` and `WorktreeCallsTests.swift` cover it.

| Path and lines | Lines |
| --- | ---: |
| `apps/macos/Sources/FarCooler/ResumeBranch.swift` | 276 |
| `apps/macos/Sources/FarCooler/DaemonClient.swift` 1872–1896 | 25 |
| `apps/macos/Sources/FarCooler/ContentView+Starts.swift` 11–32 | 22 |
| `apps/macos/Sources/FarCooler/ContentView.swift` 121, 673–683 | 12 |
| `apps/macos/Sources/FarCooler/ContentView+Commands.swift` 313–319 | 7 |
| `apps/macos/Sources/FarCooler/Commands.swift` 17, 151 | 2 |

### B3. Should editing runner themes be Mac-only? — 329 lines

Phone Settings → runner → Themes on This Runner opens `ThemeEditorView`.
Save and delete call `theme.upsert` and `theme.delete` as an authorized
runner administrator. The Mac's `RunnerSettings` and `RunnerSettingsStore`
already provide runner theme editing. Dropping the phone surface would
lose in-place color edits, preview, duplicate and delete on a phone. The
jobs document limits phone Tend to pairing. Theme selection and normal
rendering on the phone still serve readability, so leave the theme picker and
runner theme reads used by it. Pairing, runner login and team access are
separate paths.

| Path and lines | Lines |
| --- | ---: |
| `apps/ios/FarCooler/ThemeEditorView.swift` | 198 |
| `apps/ios/FarCooler/RunnerSettings.swift` 15, 75, 79, 276–295, 331, 426–478, 502–508, 633–648, 651–668 | 118 |
| `apps/ios/FarCooler/Connection.swift` 1971–1983 | 13 |

### B4. Do the standalone Plan widgets earn a place alongside Fleet? — 216 lines

The two WidgetKit bundles register these widgets. Their providers fetch a
relay pulse and draw the selected workspace's Needs You, Now and Next. The
jobs document gives phone widgets and complications the fleet-wide Reassure
job. Fleet widgets and complications still provide the fleet glance. The
phone's Plan and the Plan section in the watch app still show the focused
plan when opened. Removing these registrations would lose a separate
workspace Plan in the iOS widget gallery and on the watch face and Smart
Stack, including installations that currently use it. The shared Plan
presentation and pulse pieces have other callers, including
`FleetListView` and `AgentActivityWidget`, so their lines are excluded.

| Path and lines | Lines |
| --- | ---: |
| `apps/ios/FarCoolerActivity/PlanWidget.swift` | 117 |
| `apps/ios/FarCoolerWatchWidgets/WatchPlanWidget.swift` | 97 |
| `apps/ios/FarCoolerActivity/FarCoolerActivityBundle.swift` 20 | 1 |
| `apps/ios/FarCoolerWatchWidgets/WatchFleetWidget.swift` 16 | 1 |

### B5. Should phone Changes keep offering to open a manual GitHub PR? — 34 lines

`ShellScreen.readPullRequest` reads `stack.get` for the visible Changes
pane. On a repo for which the runner knows there is no PR, the header opens
a GitHub compare link that starts a manual PR. Existing PRs have a separate
status and link, and the Changes view remains central to Review. The engine
already owns the configured review and landing path for managed tickets.
Removing the create offer would take away the phone's shortcut for opening
a PR on a branch outside that path as well. Keep the existing-PR row,
read route and diff. The `changes-pr-create` cases in
`apps/ios/FarCoolerUITests/ChangesPullRequestTests.swift` document that behavior.

| Path and lines | Lines |
| --- | ---: |
| `apps/ios/FarCooler/ChangesView.swift` 604–610, 679–705 | 34 |

**Total for B · 1,442 conditional lines** across five owner decisions.

## Uncertain or still in use

- **Legacy landing cards and status views.** Mac `PlanSectionsView.swift`
  and phone `PlanSectionRows.swift` render ordinary landing cards alongside
  engine landing groups. `PlanLandings.swift` also feeds CI and historical
  page lookups. The engine's queue groups do not establish parity for old
  landings or old runners. Determine supported runner and history behavior
  before counting these files. The older iPad Tasks/Plan switch also has an
  explicit toolbar route through `WorkspaceScreen.boardList` and honors
  saved `PlanChoice`. The newer column alone does not prove the Tasks
  list's Unread and history affordances are redundant.
- **Mac standalone diff renderer.** `apps/macos/Sources/FarCooler/DiffView.swift`
  has no product constructor in the source search, and Changes uses its
  own row renderer. It has consumers in capture and specimen suites and
  `Tests/RenderWiringTests/DiffViewCacheTests.swift`. Verify equivalent
  coverage on the live diff before retiring the renderer and those tests.
- **Watch developer capture flags.** `WatchCaptureScenes.swift` is called
  by the watch entry point under DEBUG. The flags are documented for a
  simulator launch, which may be supplied outside the repository. A missing
  in-repo setter is insufficient evidence. The watch's production
  `WatchRoute` destinations all have real producers.
- **Compatibility, setup and transport.** Phone migration removes old
  composer draft defaults for existing installs. AgentKit's wire decoders and
  bridge models include older-runner fallbacks. Enrollment, permission
  delivery, watch prefetch and the notification service exercise indirect
  OS and wire entry points. Those remain outside the counts, as does any
  deeper Rust port of live Swift presentation behavior.

## Validation

This is a report-only change. No product tests are affected. CI classified the actual report path as report-only.
The classifier self-test passed, along with the shared-core and charter-intercept lints. The NativeRowsFixture
generator passed its two Python tests. The production membership checker
passed on a scratch project rendered by the iOS generator's project
template with icon labeling bypassed. The direct attempt hit a sandboxed
Swift module-cache write in the unrelated icon step. No native app build
or simulator run was used. The report checks confirmed every line range,
subtotal, overall total, and the absence of overlapping slices.
