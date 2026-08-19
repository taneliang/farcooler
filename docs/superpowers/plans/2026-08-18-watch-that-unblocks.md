# Watch That Unblocks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An Apple Watch app that shows the fleet, and lets you dictate a prompt or answer a waiting agent from your wrist.

**Architecture:** The watch holds no SSH identity. It renders a `FleetSnapshot` the phone sends by `updateApplicationContext`, and performs actions by `sendMessage` to the phone, which executes them through the code paths the phone already uses. Two new generated targets: a watchOS app and its widget extension.

**Tech Stack:** Swift 6 / SwiftUI / WatchConnectivity / WidgetKit, Swift Testing, Python 3 (`generate-project.py`).

**Spec:** `docs/superpowers/specs/2026-08-18-watch-that-unblocks-design.md`
**Blueprint for the target generation:** `.superpowers/sdd/2026-08-17-glance-surfaces/task-0-report.md` — the spike that already built one. Read it before Task 2; it names every load-bearing setting and the one environment requirement.

## Global Constraints

- **US English** in all code, comments and copy. Never "authorise", "colour", "centre".
- **Apple copy conventions:** title-style capitalization for buttons and titles, sentence case for body, contractions, typographic apostrophes (’).
- **Say *runner*, not *machine* or *host*,** for the thing a device connects to. `AgentActivityAttributes.machine` and `FleetSnapshot.Agent.machine` keep their names — the relay matches the first by field name — but prose and new user-visible copy say runner.
- **A client never re-derives what the daemon decided.** `glyph`, `headline`, `line`, `rank` and the ordering come off the snapshot unchanged. Re-deriving any of them is the failure the ladder exists to prevent.
- **Fields added to a wire or stored type are OPTIONAL, never defaulted.** Swift's synthesized `Decodable` ignores defaults and throws on a missing key.
- **Never `git add apps/ios/FarCooler.xcodeproj/project.pbxproj`.** CI regenerates it (`ci.yml`, `canary.yml`, `release.yml`) and states it is "never committed as the source of truth". Always run `python3 apps/ios/generate-project.py` before building.
- **The build machine needs the watchOS Simulator platform.** `xcodebuild -downloadPlatform watchOS`. Already installed on this machine. Without it a `-scheme` build refuses with no `error:` line.
- iOS floor 26.0; watchOS floor 26.0; AgentKit floors macOS 26 / iOS 26 / watchOS 26.
- Run all commands from the worktree root: `/Users/e-liang/Dev/overnight/.claude/worktrees/watch-and-widgets`.
- Commit messages lowercase-prefixed, ending with:
  `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`

## File Structure

| File | Responsibility |
| --- | --- |
| `apps/shared/AgentKit/Package.swift` | *(modify)* Add the watchOS platform |
| `apps/shared/AgentKit/Sources/AgentKit/WatchLink.swift` | *(create)* The request/reply vocabulary both sides share, and its dictionary coding |
| `apps/shared/AgentKit/Tests/AgentKitTests/WatchLinkTests.swift` | *(create)* Round-trip and rejection tests |
| `apps/ios/generate-project.py` | *(modify)* Two watch targets, their embed phase, dependency pair, App Group |
| `apps/ios/FarCoolerWatch/FarCoolerWatchApp.swift` | *(create)* Entry point |
| `apps/ios/FarCoolerWatch/WatchLinkClient.swift` | *(create)* `FleetClient` and its WatchConnectivity implementation |
| `apps/ios/FarCoolerWatch/FleetListView.swift` | *(create)* The fleet, by rank |
| `apps/ios/FarCoolerWatch/AgentDetailView.swift` | *(create)* One agent, its feed, its actions |
| `apps/ios/FarCoolerWatch/ComposeView.swift` | *(create)* Dictation to prompt |
| `apps/ios/FarCoolerWatch/PermissionView.swift` | *(create)* The agent's own options |
| `apps/ios/FarCooler/WatchLinkHost.swift` | *(create)* The phone's half: answers requests, pushes context |
| `apps/ios/FarCoolerWatchWidgets/WatchFleetWidget.swift` | *(create)* Complication and Smart Stack |
| `scripts/generator-test.sh` | *(create)* Asserts the load-bearing generated settings |

---

### Task 1: The shared vocabulary, and AgentKit on watchOS

Both sides must agree on the message shape down to the key names, for the same reason `AgentActivityAttributes` lives in one file: a renamed key on one side does not fail to build, it fails to arrive.

**Files:**
- Modify: `apps/shared/AgentKit/Package.swift`
- Create: `apps/shared/AgentKit/Sources/AgentKit/WatchLink.swift`
- Test: `apps/shared/AgentKit/Tests/AgentKitTests/WatchLinkTests.swift`

**Interfaces:**
- Consumes: `FleetSnapshot` (already in AgentKit).
- Produces: `WatchRequest` (`.prompt`, `.answer`, `.pendingPermission`), `WatchReply` (`.sent`, `.failed(String)`, `.permission(WatchPermission?)`), `WatchPermission` (`id`, `toolCall`, `options: [WatchPermissionOption]`), each with `init?(dictionary:)` and `var dictionary: [String: Any]`. Tasks 3, 4 and 5 consume all of these.

- [ ] **Step 1: Add the platform**

In `apps/shared/AgentKit/Package.swift`, extend `platforms` with `.watchOS("26.0")`. Keep the existing comment's shape and add a sentence saying why watchOS is here — the watch renders the same snapshot the phone does, from the same code, so the two cannot disagree about staleness.

- [ ] **Step 2: Write the failing tests**

Create `apps/shared/AgentKit/Tests/AgentKitTests/WatchLinkTests.swift`. Cover, at minimum:
- each `WatchRequest` case round-trips through `dictionary` → `init?(dictionary:)`;
- each `WatchReply` case round-trips;
- a dictionary with an unknown `kind` returns nil rather than crashing or guessing;
- a dictionary missing a required key returns nil;
- every value in a produced dictionary is a property-list type (`String`, `Int`, `Double`, `Bool`, `Data`, `Date`, `Array`, `Dictionary`) — **this is the one that matters**: `WCSession` rejects anything else at runtime, and the failure is an action that silently never happens. Write it as a recursive check over the produced dictionary, not a spot check.

- [ ] **Step 3: Run them and watch them fail**

```bash
swift test --package-path apps/shared/AgentKit --filter WatchLinkTests
```
Expected: FAIL, `WatchRequest` not in scope.

- [ ] **Step 4: Write `WatchLink.swift`**

Dictionary coding by hand rather than `Codable`: `WCSession` carries a property-list dictionary, not `Data`, and hand-written coding is what lets an unknown `kind` decode to nil instead of throwing. Document that reason in the file header.

Keep the vocabulary tiny — three requests, three replies. Every string key is a constant in one place.

- [ ] **Step 5: Run the tests**

```bash
swift test --package-path apps/shared/AgentKit
```
Expected: all pass, including the 92 that already exist.

- [ ] **Step 6: Commit**

---

### Task 2: The watchOS app target

**Read `.superpowers/sdd/2026-08-17-glance-surfaces/task-0-report.md` first.** A spike already did this and recorded what broke. You are making its throwaway permanent.

**Files:**
- Modify: `apps/ios/generate-project.py`
- Create: `apps/ios/FarCoolerWatch/FarCoolerWatchApp.swift`
- Create: `scripts/generator-test.sh`

**Interfaces:**
- Consumes: the spike's recorded settings.
- Produces: a `FarCoolerWatch` target whose product is embedded in the app at `Watch/`, and `WATCH_SOURCES` for later tasks to add files to.

- [ ] **Step 1: Add the target**

Mirror `activityTarget` object for object, with the spike's settings: `productType = "com.apple.product-type.application"`, `SDKROOT = watchos`, `WATCHOS_DEPLOYMENT_TARGET = 26.0`, `TARGETED_DEVICE_FAMILY = 4`, `PRODUCT_BUNDLE_IDENTIFIER = {BUNDLE_ID}.watchkitapp`, `GENERATE_INFOPLIST_FILE = YES`, `INFOPLIST_KEY_WKCompanionAppBundleIdentifier = {BUNDLE_ID}`, `FARCOOLER_APP_GROUP`, and the `FarCoolerAppGroup` Info.plist key via `INFOPLIST_KEY_FarCoolerAppGroup`.

The embed phase is its **own** `PBXCopyFilesBuildPhase` with `dstSubfolderSpec = 16` and `dstPath = "$(CONTENTS_FOLDER_PATH)/Watch"` — not the extensions' `13`/`PlugIns` phase. Add the `PBXTargetDependency`/`PBXContainerItemProxy` pair.

`FleetSnapshot.swift`, `SnapshotStore.swift` and `WatchLink.swift` compile into this target too, which needs a fourth set of build ids (`watch_build_ids`) — a build id reused across two sources phases fails with "multiple commands produce".

- [ ] **Step 2: A trivial app that builds**

`FarCoolerWatchApp.swift`: `@main struct FarCoolerWatchApp: App` rendering a placeholder. Later tasks replace the body.

- [ ] **Step 3: Write the generator test**

`scripts/generator-test.sh`, in the style of `scripts/version-test.sh` and `scripts/icon-test.sh`. Generate the project and assert on the emitted `project.pbxproj`:
- the watch app is embedded at `dstSubfolderSpec = 16` with `dstPath` containing `Watch`;
- its bundle id is the app's plus `.watchkitapp`, and the widgets' is that plus `.widgets` (add once Task 6 lands);
- every target that reads the snapshot carries `FARCOOLER_APP_GROUP`, and all values agree;
- the extension carries `FARCOOLER_URL_SCHEME`.

Each of these fails after the build rather than during it, which is why they are worth asserting mechanically.

- [ ] **Step 4: Build**

```bash
python3 apps/ios/generate-project.py
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination "generic/platform=iOS" ARCHS=arm64 ONLY_ACTIVE_ARCH=NO \
  CODE_SIGNING_ALLOWED=NO build
```
`error:` lines are the signal. Then confirm the watch app is really embedded by inspecting the built product, not the project text:
```bash
find apps/ios/build -name "*.app" -path "*Watch*"
```

- [ ] **Step 5: Run the generator test, then commit**

Commit the generator, the Swift file and `scripts/generator-test.sh`. Not the pbxproj.

---

### Task 3: The transport, both sides

**Files:**
- Create: `apps/ios/FarCoolerWatch/WatchLinkClient.swift`
- Create: `apps/ios/FarCooler/WatchLinkHost.swift`
- Modify: `apps/ios/FarCooler/FleetSnapshotWriter.swift` (also hand the snapshot to the watch)
- Modify: `apps/ios/generate-project.py` (`SOURCES`, `WATCH_SOURCES`)

**Interfaces:**
- Consumes: `WatchRequest`/`WatchReply` (Task 1), `FleetSnapshot`, `SnapshotStore`.
- Produces: `FleetClient` protocol (`var state: WatchState { get }`, `func send(_:) async -> WatchReply`), `WatchState` enum (`.live(FleetSnapshot)`, `.cached(FleetSnapshot)`, `.nothing`), and `WatchLinkClient` conforming to it. Tasks 4, 5 and 6 consume these.

- [ ] **Step 1: The watch side**

`WatchLinkClient`: a `WCSessionDelegate` that stores each received context through `SnapshotStore.write(_:)` so the complication can read it too, publishes `WatchState`, and sends requests.

`WatchState` is the spec's three reachability states and must be computed from `WCSession.isReachable` plus whether a snapshot exists — never from whether data merely looks recent.

- [ ] **Step 2: The phone side**

`WatchLinkHost`: activates a `WCSession` on the phone, answers `WatchRequest`s, and exposes `send(snapshot:)`. It performs a prompt through the same composer path the phone's own UI uses and a permission through `{terminal, requestId, optionId}` — locate both and call them rather than reimplementing.

`pendingPermission` reads the phone's live transcript for that terminal. If the phone has no stream, reply `.permission(nil)` — "nothing pending" is the honest answer, and the watch renders it as such.

- [ ] **Step 3: Push the snapshot**

In `FleetSnapshotWriter.write(fleet:machine:)`, after `SnapshotStore.write`, hand the same snapshot to `WatchLinkHost`. One projection, two consumers — the widgets and the watch — so they cannot disagree.

- [ ] **Step 4: Build both, then commit**

---

### Task 4: The fleet and the agent

**Files:**
- Create: `apps/ios/FarCoolerWatch/FleetListView.swift`
- Create: `apps/ios/FarCoolerWatch/AgentDetailView.swift`
- Modify: `apps/ios/FarCoolerWatch/FarCoolerWatchApp.swift`, `apps/ios/generate-project.py`

**Interfaces:**
- Consumes: `FleetClient`, `WatchState`, `FleetSnapshot`.
- Produces: the screens Task 5 pushes its actions from.

- [ ] **Step 1: The fleet list**

Rows of `glyph`, `headline`, `line`, in `snapshot.ranked` order. Do not sort. Apply `confidence(in:at:)` with the prefix degradation spec 1 settled — "last seen …" before the headline, never after, because text truncates at the tail and a suffix marker vanishes from exactly the row too narrow to keep it.

When `state` is `.cached`, say so once at the top with the snapshot's age, in sentence case. When `.nothing`, an empty state that names the phone.

- [ ] **Step 2: The agent detail**

Headline, signal line, elapsed time from `activityChangedAt`, `feed`'s three lines, subagent names. Buttons for Task 5's screens, disabled unless `state` is `.live`.

- [ ] **Step 3: Build, then commit**

---

### Task 5: Dictating, and answering

**Files:**
- Create: `apps/ios/FarCoolerWatch/ComposeView.swift`
- Create: `apps/ios/FarCoolerWatch/PermissionView.swift`
- Modify: `apps/ios/FarCoolerWatch/AgentDetailView.swift`, `apps/ios/generate-project.py`

**Interfaces:**
- Consumes: `FleetClient.send(_:)`, `WatchPermission`.
- Produces: nothing later tasks consume.

- [ ] **Step 1: Compose**

A watchOS `TextField` gives dictation and Scribble for free. On send, show an in-flight state — the phone may be asleep and `sendMessage` may take real time (risk 1), and a button that looks inert is a prompt someone sends twice. Report `.failed` in a sentence, never a raw error.

- [ ] **Step 2: Permission**

Ask `pendingPermission` when the screen opens. Render the agent's own option names as buttons; never a hardcoded Allow/Deny pair. `.permission(nil)` renders as "Nothing to answer" — the agent is blocked on something that is not a permission — with the signal line still visible so the person can see what it is.

- [ ] **Step 3: Build, then commit**

---

### Task 6: The complication and the Smart Stack

**Files:**
- Create: `apps/ios/FarCoolerWatchWidgets/WatchFleetWidget.swift`
- Modify: `apps/ios/generate-project.py`, `scripts/generator-test.sh`

**Interfaces:**
- Consumes: `SnapshotStore.read()`, `FleetSnapshot`.
- Produces: the finished surfaces.

- [ ] **Step 1: The target**

`{BUNDLE_ID}.watchkitapp.widgets`, embedded in the **watch app's** PlugIns, not the phone's. A fifth set of build ids.

- [ ] **Step 2: The widget**

`accessoryCircular` shows `glyph` and the count needing you; `accessoryRectangular` shows `headline` and `line`; `accessoryInline` shows `headline`. `rank` picks which agent. Reuse spec 1's `FleetWidget` decisions wholesale — including the timeline that schedules an entry per distinct staleness moment so the watch can go stale on its own, and the "nothing ever written" state that must not read as an empty fleet.

- [ ] **Step 3: Extend the generator test, build, commit**

---

## Verification

- `swift test --package-path apps/shared/AgentKit` — the existing 92 plus Task 1's.
- `bash scripts/generator-test.sh` — the load-bearing generated settings.
- The `xcodebuild` command above — zero `error:` lines.
- Nothing here has run on hardware. Collect device steps for a hand-back: whether a real paired watch accepts the companion relationship, whether provisioning nests for two new bundle ids, cold-phone `sendMessage` latency, and how often `updateApplicationContext` actually lands.
