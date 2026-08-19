# Glance Surfaces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Put the daemon's compact-rendering ladder on three iOS surfaces — a Live Activity that follows a whole run, a home screen widget, and a lock screen widget — so a fleet can be overseen from a locked phone.

**Architecture:** The daemon already derives and serializes the ladder (`glyph`, `headline`, `line`, `rank`, `feed`, `turnStartedAt`), and the Rust client core already projects all of it into the JSON the phone decodes. So this is almost entirely Swift plus two small changes on the host: iOS learns to decode the ladder, a `FleetSnapshot` in AgentKit is written to a per-channel App Group by the app and by a notification service extension, and every surface renders that snapshot. The daemon additionally starts pushing `working` so a card can exist for a whole run rather than only while an agent is blocked.

**Tech Stack:** Swift 6 / SwiftUI / WidgetKit / ActivityKit / UserNotifications, Swift Testing (`import Testing`), Python 3 (`generate-project.py`), Rust (daemon), TypeScript (Cloudflare Workers relay).

## Global Constraints

- **US English** in all code, comments and copy. Never "authorise", "colour", "centre".
- **Apple copy conventions:** title-case buttons, contractions, "machine" not "host", never a raw Rust error in the UI.
- **Never run `cargo fmt`.** The Rust tree is hand-formatted and CI skips the check deliberately.
- **`cargo` is not on `PATH`.** Prefix with `PATH="$HOME/.cargo/bin:$PATH"` for any Rust step.
- **Four channels.** stable / canary / preview / local each have their own bundle identifier, keychain group, URL scheme and relay. Anything new that is per-app must be per-channel, derived from one definition in `generate-project.py` — never a second list.
- **Stable keeps every bare name.** `com.farcooler.ios`, scheme `farcooler`. Existing installs must not migrate.
- **Fields added to a wire type are OPTIONAL, never defaulted.** Swift's synthesized `Decodable` throws on a missing key, so a phone meeting an older daemon would fail to decode the entire fleet over one absent field.
- **iOS deployment target is 26.0.** AgentKit's floor is `.iOS("26.0")`.
- Run all commands from the worktree root: `/Users/e-liang/Dev/overnight/.claude/worktrees/watch-and-widgets`.
- Never write generated output into a tracked path — `version.sh channel` answers `local` for a dirty tree, and every later build step then believes it is building `local`.

## File Structure

| File | Responsibility |
| --- | --- |
| `apps/ios/FarCooler/Model.swift` | *(modify)* Decode the ladder fields the core already sends |
| `apps/ios/FarCooler/FleetView.swift` | *(modify)* Order terminals by `rank`; render `line` |
| `apps/shared/AgentKit/Sources/AgentKit/FleetSnapshot.swift` | *(create)* The snapshot type, its staleness rules, its merge |
| `apps/shared/AgentKit/Sources/AgentKit/SnapshotStore.swift` | *(create)* Atomic read/write in the App Group container |
| `apps/shared/AgentKit/Tests/AgentKitTests/FleetSnapshotTests.swift` | *(create)* Coding, staleness, merge, completeness |
| `apps/ios/FarCooler/FarCooler.entitlements` | *(modify)* Add the App Group |
| `apps/ios/FarCoolerActivity/FarCoolerActivity.entitlements` | *(create)* The extension's App Group |
| `apps/ios/FarCoolerNotify/NotificationService.swift` | *(create)* Merge one agent from a push, reload timelines |
| `apps/ios/FarCoolerNotify/Info.plist` | *(create)* Marks it a notification service extension |
| `apps/ios/FarCoolerNotify/FarCoolerNotify.entitlements` | *(create)* The extension's App Group |
| `apps/ios/generate-project.py` | *(modify)* `FARCOOLER_APP_GROUP`, the scheme for the activity target, the new extension target |
| `apps/ios/FarCoolerActivity/AgentActivityWidget.swift` | *(modify)* Elapsed timer, signal line, channel-correct deep link |
| `apps/ios/FarCoolerActivity/FleetWidget.swift` | *(create)* Home and lock screen widgets |
| `apps/ios/FarCoolerActivity/FarCoolerActivityBundle.swift` | *(modify)* Register the new widget |
| `apps/shared/AgentKit/Sources/AgentKit/AgentActivityAttributes.swift` | *(modify)* `startedAt` on attributes, `line`/`feed` on state |
| `apps/ios/FarCooler/FarCoolerApp.swift` | *(modify)* Route `…://terminal/<id>` |
| `crates/daemon/src/watch.rs` | *(modify)* Push `working` for a card, throttled by tier |
| `services/relay/src/push.ts` | *(modify)* `status`/`label`/`mutable-content` in the alert; priority by urgency |
| `services/relay/src/index.ts` | *(modify)* Accept `working` as a card-only update |

---

### Task 0: Prove a watchOS target can be generated (throwaway spike)

Specs 2 and 3 rest on `generate-project.py` growing a watchOS app. This answers whether that is an afternoon or a week, while the answer can still change the plan. **Nothing in this plan depends on it, and the code is deleted at the end.**

**Files:**
- Modify (temporarily): `apps/ios/generate-project.py`

**Interfaces:**
- Consumes: nothing.
- Produces: nothing. Its only output is the note written in Step 5.

- [ ] **Step 1: Add a minimal watchOS app target to the generator**

Work on a scratch copy so the tracked file is never dirty:

```bash
cp apps/ios/generate-project.py /tmp/spike-generate.py
```

Add to the scratch copy, mirroring how `activityTarget` is built: a `PBXNativeTarget` of `productType = "com.apple.product-type.application"`, `SDKROOT = watchos`, `WATCHOS_DEPLOYMENT_TARGET = 26.0`, `PRODUCT_BUNDLE_IDENTIFIER = {BUNDLE_ID}.watchkitapp`, `INFOPLIST_KEY_WKCompanionAppBundleIdentifier = {BUNDLE_ID}`, `TARGETED_DEVICE_FAMILY = 4`, and a `PBXCopyFilesPhase` with `dstSubfolderSpec = 16` and `dstPath = "$(CONTENTS_FOLDER_PATH)/Watch"` embedding it in the app.

Give it one source file with a trivial `@main struct SpikeApp: App`.

- [ ] **Step 2: Generate and build**

```bash
python3 /tmp/spike-generate.py
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination "generic/platform=iOS" ARCHS=arm64 ONLY_ACTIVE_ARCH=NO \
  CODE_SIGNING_ALLOWED=NO build 2>&1 | grep -E "error:|BUILD"
```

Expected: either a clean build, or specific errors. `error:` lines are the signal, not the final status line.

- [ ] **Step 3: Install on hardware**

Follow the device recipe in `docs/releasing.md`: `./scripts/build-ios-frameworks.sh --device` first, then `xcodebuild … -allowProvisioningUpdates CODE_SIGN_STYLE=Automatic DEVELOPMENT_TEAM=H6A2TRW47J CODE_SIGN_IDENTITY="Apple Development"`, then `xcrun devicectl device install app`. Confirm the watch app appears on the paired watch.

- [ ] **Step 4: Revert every change**

```bash
git checkout apps/ios/generate-project.py
rm /tmp/spike-generate.py
python3 apps/ios/generate-project.py
git status --short   # expected: clean, or only the regenerated pbxproj matching HEAD
```

- [ ] **Step 5: Write down what was learned**

Append a short section to `docs/superpowers/specs/2026-08-17-glance-surfaces-design.md` under "Risks", replacing risk 1 with what is now known: how long it took, which settings were load-bearing, and anything that failed only on device. Specs 2 and 3 are written from this.

```bash
git add docs/superpowers/specs/2026-08-17-glance-surfaces-design.md
git commit -m "docs: record what the watchOS target spike found"
```

---

### Task 1: iOS decodes the ladder

The Rust core already serializes `glyph`, `headline`, `line`, `rank`, `feed`, `subagents`, `blockedQuestion` and `turnStartedAt` (`crates/client/src/session.rs:288-341`). iOS decodes none of them. No Rust change and no xcframework rebuild — this is pure Swift.

**Files:**
- Modify: `apps/ios/FarCooler/Model.swift` (the `Terminal` struct, after `turnFailed`)
- Modify: `apps/ios/FarCooler/FleetView.swift:765`
- Test: `apps/ios/FarCoolerUITests/` — none; covered by Task 2's pure tests and by on-device check in Step 5

**Interfaces:**
- Consumes: the JSON at `crates/client/src/session.rs:288-341`.
- Produces: `Terminal.glyph: String?`, `Terminal.headline: String?`, `Terminal.line: String?`, `Terminal.rank: UInt32?`, `Terminal.feed: [String]?`, `Terminal.subagents: [String]?`, `Terminal.blockedQuestion: String?`, `Terminal.turnStartedAt: Double?`, and `Terminal.signalLine: String`. Tasks 4 and 9 read these.

- [ ] **Step 1: Add the fields to `Terminal`**

In `apps/ios/FarCooler/Model.swift`, immediately after `var turnFailed: Bool?`:

```swift
    /// Unix milliseconds when the current turn started, or nil between turns.
    ///
    /// Held across Blocked on the host: approving a tool call does not begin a
    /// new turn, so a card's timer does not restart when you answer one.
    var turnStartedAt: Double?
    /// What the agent is asking, while it is asking it.
    var blockedQuestion: String?
    /// The last few things the agent SAID, oldest first, at most three.
    ///
    /// A transcript and only a transcript — the agent's own prose, with no verb
    /// in front of it. What it DID arrives on `line` instead. Already redacted
    /// and cut to a row's width by the daemon, so this app renders them and
    /// decides nothing about them.
    ///
    /// Optional because a daemon from before this existed sends no key, and a
    /// row with no feed must read as "nothing to say" rather than as a decoding
    /// failure that takes the whole fleet down.
    var feed: [String]?
    /// Where the agent is, in one line: the question it is blocked on, its
    /// position in its own task list, or what it is doing right now.
    ///
    /// One rung of the daemon's compact ladder. The priority between those is
    /// decided on the host — see `farcooler_core::feed::line` — because a Mac,
    /// a phone and a watch deciding it separately is three surfaces disagreeing
    /// about one pane.
    var line: String?
    /// The state in one character: `?` blocked, `●` working, `✓` done, `✗`
    /// failed, `·` idle. The narrowest rung, for a lock screen accessory.
    var glyph: String?
    /// The state plus just enough to say whose, at most ~18 characters.
    var headline: String?
    /// Where this terminal sorts in a fleet view. SMALLER sorts FIRST: blocked
    /// outranks done outranks working, and within a tier the oldest first.
    ///
    /// Computed on the host beside `activity`, so a widget showing one agent
    /// and this list showing twelve agree about which one matters.
    var rank: UInt32?
    /// The agents this agent spawned and has not finished with, named.
    /// Their COUNT is already inside `line`; these are the names.
    var subagents: [String]?
```

- [ ] **Step 2: Add the one derived property**

After `var canSwitchPaneMode: Bool { chatCapable == true }`:

```swift
    /// The signal line, or empty when the host has nothing to say.
    ///
    /// Trimmed here rather than at each call site: a line that is whitespace is
    /// a line that draws a blank row and makes every surface taller for
    /// nothing, and three surfaces trimming it separately is three chances to
    /// forget.
    var signalLine: String {
        (line ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Where this terminal sorts. Absent `rank` sorts last: a daemon too old to
    /// send one is a daemon that cannot tell us this pane is urgent, and
    /// guessing that it is would put an unknown above a known blocked agent.
    var sortRank: UInt32 { rank ?? UInt32.max }
```

- [ ] **Step 3: Order the fleet by rank**

In `apps/ios/FarCooler/FleetView.swift:765`, change:

```swift
                    ForEach(workspace.terminals) { terminal in
```

to:

```swift
                    ForEach(workspace.terminals.sorted { $0.sortRank < $1.sortRank }) { terminal in
```

Make the same change at line 847, inside the `ForEach(hidden)` block.

- [ ] **Step 4: Build**

```bash
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination "generic/platform=iOS" ARCHS=arm64 ONLY_ACTIVE_ARCH=NO \
  CODE_SIGNING_ALLOWED=NO build 2>&1 | grep -E "error:|BUILD"
```

Expected: no `error:` lines.

- [ ] **Step 5: Verify against a live daemon**

Run the app against a machine with an agent working. The fleet list must reorder so a blocked agent rises to the top of its workspace. If nothing reorders, the daemon is older than the ladder — check `farcooler --version` on that machine before assuming this task is wrong.

- [ ] **Step 6: Commit**

```bash
git add apps/ios/FarCooler/Model.swift apps/ios/FarCooler/FleetView.swift
git commit -m "feat(ios): read the ladder the daemon already sends"
```

---

### Task 2: `FleetSnapshot` and its rules

The pure core of every surface, testable with no device and no daemon.

**Files:**
- Create: `apps/shared/AgentKit/Sources/AgentKit/FleetSnapshot.swift`
- Test: `apps/shared/AgentKit/Tests/AgentKitTests/FleetSnapshotTests.swift`

**Interfaces:**
- Consumes: nothing.
- Produces: `FleetSnapshot` with `agents: [FleetSnapshot.Agent]`, `capturedAt: Date`, `complete: Bool`; `FleetSnapshot.empty`; `FleetSnapshot.staleAfter: TimeInterval`; `FleetSnapshot.ranked: [Agent]`; `FleetSnapshot.confidence(in:at:) -> Confidence` with cases `.known` and `.lastSeen`; `FleetSnapshot.merging(_:at:) -> FleetSnapshot`; `FleetSnapshot.needingYou: Int`. Tasks 3, 4, 5 and 9 all consume these.

- [ ] **Step 1: Write the failing tests**

Create `apps/shared/AgentKit/Tests/AgentKitTests/FleetSnapshotTests.swift`:

```swift
import Foundation
import Testing

@testable import AgentKit

/// The rules every glance surface renders by.
///
/// Pure on purpose: a widget cannot be stepped through in a debugger and a
/// lock screen cannot be asserted against, so the decisions they make live
/// here where they can be.
struct FleetSnapshotTests {
    private func agent(
        _ id: String,
        status: String,
        rank: UInt32 = 0
    ) -> FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: id, label: "claude", machine: "orchard", status: status,
            glyph: "●", headline: "claude 4m", line: "Writing fruit.txt",
            feed: ["Reading watch.rs."], rank: rank, turnFailed: false,
            activityChangedAt: nil)
    }

    @Test func aSnapshotRoundTripsThroughJson() throws {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working")],
            capturedAt: Date(timeIntervalSince1970: 1_000_000),
            complete: true)
        let data = try JSONEncoder().encode(snapshot)
        #expect(try JSONDecoder().decode(FleetSnapshot.self, from: data) == snapshot)
    }

    /// A newer daemon's extra key must not take the whole snapshot down — the
    /// same rule the wire types follow, for the same reason.
    @Test func anUnknownKeyDoesNotFailTheDecode() throws {
        let json = """
        {"agents":[{"id":"t1","label":"claude","machine":"orchard",
        "status":"working","glyph":"●","headline":"claude 4m","line":"x",
        "feed":[],"rank":0,"turnFailed":false,"somethingNewer":42}],
        "capturedAt":1000000,"complete":true}
        """
        let snapshot = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        #expect(snapshot.agents.count == 1)
    }

    /// Blocked and done are LATCHED: an agent waiting on you is still waiting
    /// an hour later, and nothing but a person changes that.
    @Test func aLatchedStatusStaysConfidentWhenOld() {
        let old = Date(timeIntervalSince1970: 0)
        let now = old.addingTimeInterval(FleetSnapshot.staleAfter + 1)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked"), agent("t2", status: "done")],
            capturedAt: old, complete: true)
        #expect(snapshot.confidence(in: snapshot.agents[0], at: now) == .known)
        #expect(snapshot.confidence(in: snapshot.agents[1], at: now) == .known)
    }

    /// Working is VOLATILE: an agent working an hour ago has very likely
    /// finished, and a widget that keeps asserting it is working is lying.
    @Test func aWorkingStatusDegradesWhenOld() {
        let old = Date(timeIntervalSince1970: 0)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working")], capturedAt: old, complete: true)
        #expect(
            snapshot.confidence(in: snapshot.agents[0], at: old.addingTimeInterval(60))
                == .known)
        #expect(
            snapshot.confidence(
                in: snapshot.agents[0],
                at: old.addingTimeInterval(FleetSnapshot.staleAfter + 1)) == .lastSeen)
    }

    @Test func agentsSortByRankSmallestFirst() {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working", rank: 900),
                     agent("t2", status: "blocked", rank: 10)],
            capturedAt: Date(), complete: true)
        #expect(snapshot.ranked.map(\.id) == ["t2", "t1"])
    }

    /// A push carries ONE agent. Merging it must not be how the other five
    /// disappear from the widget.
    @Test func mergingOneAgentKeepsTheOthers() {
        let before = FleetSnapshot(
            agents: [agent("t1", status: "working"), agent("t2", status: "working")],
            capturedAt: Date(timeIntervalSince1970: 0), complete: true)
        let now = Date(timeIntervalSince1970: 500)
        let after = before.merging(agent("t2", status: "blocked"), at: now)
        #expect(after.agents.count == 2)
        #expect(after.agents.first { $0.id == "t2" }?.status == "blocked")
        #expect(after.agents.first { $0.id == "t1" }?.status == "working")
        #expect(after.capturedAt == now)
    }

    @Test func mergingAnUnknownAgentAddsIt() {
        let before = FleetSnapshot.empty
        let after = before.merging(agent("t9", status: "blocked"), at: Date())
        #expect(after.agents.map(\.id) == ["t9"])
    }

    /// A snapshot assembled only from pushes knows about the agents that
    /// happened to notify and nothing else. Rendering it as the fleet would
    /// assert that the other five do not exist.
    @Test func aSnapshotBuiltOnlyFromPushesIsNeverComplete() {
        var snapshot = FleetSnapshot.empty
        #expect(snapshot.complete == false)
        for id in ["t1", "t2", "t3"] {
            snapshot = snapshot.merging(agent(id, status: "blocked"), at: Date())
        }
        #expect(snapshot.complete == false)
    }

    @Test func mergingIntoACompleteSnapshotKeepsItComplete() {
        let before = FleetSnapshot(
            agents: [agent("t1", status: "working")], capturedAt: Date(), complete: true)
        #expect(before.merging(agent("t1", status: "done"), at: Date()).complete)
    }

    @Test func needingYouCountsOnlyBlockedAgents() {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked"), agent("t2", status: "working"),
                     agent("t3", status: "done"), agent("t4", status: "blocked")],
            capturedAt: Date(), complete: true)
        #expect(snapshot.needingYou == 2)
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
swift test --package-path apps/shared/AgentKit --filter FleetSnapshotTests
```

Expected: FAIL — "cannot find 'FleetSnapshot' in scope".

- [ ] **Step 3: Write the implementation**

Create `apps/shared/AgentKit/Sources/AgentKit/FleetSnapshot.swift`:

```swift
import Foundation

/// The fleet as some surface last knew it.
///
/// A widget cannot hold an SSH session and neither can a watch, so every
/// surface outside the app renders whatever was last written here. Two things
/// follow from that, and both are load-bearing.
///
/// **The fields ARE the wire's fields.** `glyph`, `headline`, `line`, `rank`
/// and `feed` are copied across unchanged from what the daemon derived, never
/// recomputed. The whole point of the ladder is that it is decided once on the
/// host; a snapshot that re-derived a headline would put a widget and the app
/// into exactly the disagreement the ladder exists to prevent.
///
/// **Age is part of the value.** Nothing here is current by construction. Every
/// reader asks `confidence(in:at:)` rather than trusting `status` outright, and
/// `complete` says whether these are all the agents there are.
public struct FleetSnapshot: Codable, Sendable, Equatable {
    /// One agent, in the vocabulary the daemon already sent.
    public struct Agent: Codable, Sendable, Equatable, Identifiable {
        /// The terminal id. Named `id` for `Identifiable`, which is what lets a
        /// widget's `ForEach` be written the same way the app's is.
        public var id: String
        public var label: String
        public var machine: String
        /// `working`, `blocked` or `done`.
        ///
        /// A String rather than an enum, for the same reason
        /// `AgentActivityAttributes.ContentState.status` is one: the daemon and
        /// the relay are not Swift, and a value none of them knew about would
        /// fail to decode and take the whole snapshot down rather than one word
        /// of it.
        public var status: String
        public var glyph: String
        public var headline: String
        public var line: String
        public var feed: [String]
        public var rank: UInt32
        public var turnFailed: Bool
        /// When this state began, when the host said. Nil is "not told", which
        /// is different from "just now" and must not be rendered as it.
        public var activityChangedAt: Date?

        public init(
            id: String, label: String, machine: String, status: String,
            glyph: String, headline: String, line: String, feed: [String],
            rank: UInt32, turnFailed: Bool, activityChangedAt: Date?
        ) {
            self.id = id
            self.label = label
            self.machine = machine
            self.status = status
            self.glyph = glyph
            self.headline = headline
            self.line = line
            self.feed = feed
            self.rank = rank
            self.turnFailed = turnFailed
            self.activityChangedAt = activityChangedAt
        }

        /// Whether this status stays true as the snapshot ages.
        ///
        /// Blocked and done are facts about something that already happened and
        /// that only a person un-does. Working is a claim about right now, and
        /// right now passes.
        public var isLatched: Bool { status == "blocked" || status == "done" }
    }

    public var agents: [Agent]
    public var capturedAt: Date
    /// Whether these are all the agents there are.
    ///
    /// True only after a real fleet poll. A snapshot assembled from pushes
    /// knows about the agents that happened to notify, and a surface that drew
    /// it as the fleet would be asserting that the others do not exist.
    public var complete: Bool

    public init(agents: [Agent], capturedAt: Date, complete: Bool) {
        self.agents = agents
        self.capturedAt = capturedAt
        self.complete = complete
    }

    /// Nothing known yet. `complete` is false, which is the honest answer
    /// before anything has ever polled.
    public static let empty = FleetSnapshot(
        agents: [], capturedAt: Date(timeIntervalSince1970: 0), complete: false)

    /// How long a volatile status may be shown as current.
    ///
    /// One hour, which is `STALE_AFTER_S` in `services/relay/src/push.ts`.
    /// Deliberately the same number: two staleness thresholds with two
    /// definitions is two answers to "is this still true", and they drift.
    public static let staleAfter: TimeInterval = 60 * 60

    /// How confident a surface may sound about one agent.
    public enum Confidence: Sendable, Equatable {
        /// Render it plainly. Either the snapshot is fresh, or this status is
        /// one that stays true.
        case known
        /// Render it as the past — "last seen working" — and drop the
        /// confident styling. Do not assert the status.
        case lastSeen
    }

    public func age(at now: Date) -> TimeInterval {
        max(0, now.timeIntervalSince(capturedAt))
    }

    public func confidence(in agent: Agent, at now: Date) -> Confidence {
        if agent.isLatched { return .known }
        return age(at: now) >= Self.staleAfter ? .lastSeen : .known
    }

    /// The order every surface shows agents in.
    ///
    /// `rank` is computed on the host precisely so a complication showing one
    /// agent and a list showing twelve pick the same one.
    public var ranked: [Agent] { agents.sorted { $0.rank < $1.rank } }

    /// How many agents are waiting on a person. The number a small widget shows.
    public var needingYou: Int { agents.filter { $0.status == "blocked" }.count }

    /// Fold in the one agent a push was about, keeping every other.
    ///
    /// `complete` is carried through rather than recomputed: merging does not
    /// discover agents, so a partial snapshot stays partial however many
    /// pushes arrive, and a complete one stays complete.
    public func merging(_ agent: Agent, at now: Date) -> FleetSnapshot {
        var merged = agents
        if let index = merged.firstIndex(where: { $0.id == agent.id }) {
            merged[index] = agent
        } else {
            merged.append(agent)
        }
        return FleetSnapshot(agents: merged, capturedAt: now, complete: complete)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
swift test --package-path apps/shared/AgentKit --filter FleetSnapshotTests
```

Expected: PASS, 10 tests.

- [ ] **Step 5: Commit**

```bash
git add apps/shared/AgentKit/Sources/AgentKit/FleetSnapshot.swift \
        apps/shared/AgentKit/Tests/AgentKitTests/FleetSnapshotTests.swift
git commit -m "feat(agentkit): the fleet as a surface last knew it"
```

---

### Task 3: The per-channel App Group, and a store to write into it

**Files:**
- Create: `apps/shared/AgentKit/Sources/AgentKit/SnapshotStore.swift`
- Modify: `apps/ios/FarCooler/FarCooler.entitlements`
- Create: `apps/ios/FarCoolerActivity/FarCoolerActivity.entitlements`
- Modify: `apps/ios/FarCooler/Info.plist`
- Modify: `apps/ios/FarCoolerActivity/Info.plist`
- Modify: `apps/ios/generate-project.py`
- Test: `apps/shared/AgentKit/Tests/AgentKitTests/SnapshotStoreTests.swift`

**Interfaces:**
- Consumes: `FleetSnapshot` from Task 2.
- Produces: `SnapshotStore.groupIdentifier: String?`, `SnapshotStore.read(inGroup:) -> FleetSnapshot?`, `SnapshotStore.write(_:inGroup:) throws`, `SnapshotStore.read()`/`write(_:)` reading the group from the bundle. Tasks 4, 5 and 9 consume these.

- [ ] **Step 1: Write the failing tests**

Create `apps/shared/AgentKit/Tests/AgentKitTests/SnapshotStoreTests.swift`:

```swift
import Foundation
import Testing

@testable import AgentKit

/// Reading and writing the file every glance surface renders from.
///
/// The container is injected as a directory so these run on a Mac with no App
/// Group at all — the interesting behavior is atomicity and the shape of a
/// failure, neither of which needs a real group to exercise.
struct SnapshotStoreTests {
    private func snapshot(_ status: String) -> FleetSnapshot {
        FleetSnapshot(
            agents: [FleetSnapshot.Agent(
                id: "t1", label: "claude", machine: "orchard", status: status,
                glyph: "●", headline: "claude 4m", line: "Writing fruit.txt",
                feed: [], rank: 0, turnFailed: false, activityChangedAt: nil)],
            capturedAt: Date(timeIntervalSince1970: 42), complete: true)
    }

    @Test func aSnapshotComesBackAsItWentIn() throws {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        try SnapshotStore.write(snapshot("working"), toContainer: dir)
        #expect(SnapshotStore.read(fromContainer: dir) == snapshot("working"))
    }

    /// Nothing written yet is nil, not a throw and not an empty snapshot: a
    /// caller has to be able to tell "never polled" from "polled, found none".
    @Test func anAbsentSnapshotReadsAsNil() {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        #expect(SnapshotStore.read(fromContainer: dir) == nil)
    }

    /// A half-written file is a widget showing something that was never true.
    /// The write replaces atomically, so a reader sees the old one or the new
    /// one and never a torn one.
    @Test func aSecondWriteReplacesTheFirstWhole() throws {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        try SnapshotStore.write(snapshot("working"), toContainer: dir)
        try SnapshotStore.write(snapshot("blocked"), toContainer: dir)
        #expect(SnapshotStore.read(fromContainer: dir)?.agents.first?.status == "blocked")
    }

    /// Garbage on disk reads as nil rather than throwing into a widget's
    /// timeline provider, which has nowhere to put an error.
    @Test func anUnreadableFileReadsAsNil() throws {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        try Data("not json".utf8).write(to: dir.appendingPathComponent("fleet.json"))
        #expect(SnapshotStore.read(fromContainer: dir) == nil)
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
swift test --package-path apps/shared/AgentKit --filter SnapshotStoreTests
```

Expected: FAIL — "cannot find 'SnapshotStore' in scope".

- [ ] **Step 3: Write the store**

Create `apps/shared/AgentKit/Sources/AgentKit/SnapshotStore.swift`:

```swift
import Foundation

/// Where the snapshot lives, and the only code that reads or writes it.
///
/// A file in the App Group container rather than `UserDefaults`. An atomic
/// replace either lands or does not, so a widget reads the previous snapshot or
/// the next one and never half of one — and a half-written snapshot is a
/// surface showing a state that was never true, which is the one thing none of
/// these surfaces may do.
///
/// Every read that can fail returns nil rather than throwing. The callers are a
/// widget's timeline provider and a notification service extension, and neither
/// has anywhere useful to put an error: the honest response to an unreadable
/// snapshot is the same as to an absent one — say nothing is known.
public enum SnapshotStore {
    /// The Info.plist key each target carries, filled from the
    /// `FARCOOLER_APP_GROUP` build setting. Read rather than hardcoded because
    /// there are four channels and each has its own group; a literal here would
    /// be a second list to keep in step with `generate-project.py`.
    private static let infoKey = "FarCoolerAppGroup"

    private static let fileName = "fleet.json"

    /// This build's App Group, or nil in a target that declares none.
    public static var groupIdentifier: String? {
        guard
            let value = Bundle.main.object(forInfoDictionaryKey: infoKey) as? String,
            !value.isEmpty
        else { return nil }
        return value
    }

    /// The shared container, or nil when the entitlement is missing — which on
    /// a device means the profile did not grant the group, and is worth failing
    /// visibly in testing rather than falling back to a private directory that
    /// silently nobody else can see.
    public static func container(forGroup group: String) -> URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
    }

    public static func read(fromContainer container: URL) -> FleetSnapshot? {
        guard let data = try? Data(contentsOf: container.appendingPathComponent(fileName))
        else { return nil }
        return try? decoder.decode(FleetSnapshot.self, from: data)
    }

    public static func write(_ snapshot: FleetSnapshot, toContainer container: URL) throws {
        let data = try encoder.encode(snapshot)
        try data.write(to: container.appendingPathComponent(fileName), options: .atomic)
    }

    /// The same pair, resolving the container from this build's own group.
    /// Nil group or nil container is "nothing known", not a crash.
    public static func read() -> FleetSnapshot? {
        guard let group = groupIdentifier, let container = container(forGroup: group)
        else { return nil }
        return read(fromContainer: container)
    }

    public static func write(_ snapshot: FleetSnapshot) {
        guard let group = groupIdentifier, let container = container(forGroup: group)
        else { return }
        try? write(snapshot, toContainer: container)
    }

    /// Dates as seconds since 1970 on both sides.
    ///
    /// Pinned rather than left to the default so the app and the extension
    /// cannot be built with different strategies and write files neither can
    /// read — a failure that shows up as a permanently empty widget.
    private static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        return encoder
    }()

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        return decoder
    }()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
swift test --package-path apps/shared/AgentKit --filter SnapshotStoreTests
```

Expected: PASS, 4 tests.

- [ ] **Step 5: Define the group in the generator**

In `apps/ios/generate-project.py`, immediately after the `BUNDLE_ID = …` line:

```python
# The App Group the app and its extensions share, one per channel.
#
# The keychain names its group `$(PRODUCT_BUNDLE_IDENTIFIER)` and that trick
# does NOT transfer here. An app extension's identifier is the app's with a
# component appended, so each target would expand that expression to a
# DIFFERENT group and the app and its widget would share nothing at all — which
# presents as a widget that is permanently empty on a device where the app is
# working fine.
#
# So it is a build setting instead, set on every target that needs it and
# written into each one's Info.plist for `SnapshotStore` to read. One
# definition, four channels, no second list.
APP_GROUP = f"group.{BUNDLE_ID}"
```

Add to `TARGET_COMMON` and to `ACTIVITY_COMMON`, and add the entitlements file to `ACTIVITY_COMMON` (the extension has none today):

```
\t\t\t\tFARCOOLER_APP_GROUP = {APP_GROUP};
```

and in `ACTIVITY_COMMON` only:

```
\t\t\t\tCODE_SIGN_ENTITLEMENTS = FarCoolerActivity/FarCoolerActivity.entitlements;
```

Also add to `ACTIVITY_COMMON`, which fixes the deep link Task 8 depends on:

```
\t\t\t\tFARCOOLER_URL_SCHEME = "{version("scheme")}";
```

- [ ] **Step 6: Add the entitlement to the app**

In `apps/ios/FarCooler/FarCooler.entitlements`, before `</dict>`:

```xml
    <!-- The container the app shares with its widget and its notification
         service extension. `SnapshotStore` writes one file into it; nothing
         else uses it.

         `$(FARCOOLER_APP_GROUP)` rather than a literal, because there are four
         channels and each needs its own — a shared group would be the one
         thing canary and stable could both write to, which is exactly the
         isolation the bundle id, keychain, URL scheme and relay already have.

         Note there is no `$(AppIdentifierPrefix)` here, unlike the keychain
         group above: application-groups entitlements are not team-prefixed. -->
    <key>com.apple.security.application-groups</key>
    <array>
        <string>$(FARCOOLER_APP_GROUP)</string>
    </array>
```

- [ ] **Step 7: Give the extension its own entitlements**

Create `apps/ios/FarCoolerActivity/FarCoolerActivity.entitlements`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <!-- The extension's half of the shared container. It only ever READS: the
         snapshot is written by the app and by the notification service
         extension, and a widget that wrote would be a second author of the one
         file every surface trusts. -->
    <key>com.apple.security.application-groups</key>
    <array>
        <string>$(FARCOOLER_APP_GROUP)</string>
    </array>
</dict>
</plist>
```

- [ ] **Step 8: Carry the group into both Info.plists**

Add to `apps/ios/FarCooler/Info.plist` and `apps/ios/FarCoolerActivity/Info.plist`, before the closing `</dict>`:

```xml
    <!-- Read by `SnapshotStore`. A build setting rather than a literal so the
         four channels do not need four copies of this file. -->
    <key>FarCoolerAppGroup</key>
    <string>$(FARCOOLER_APP_GROUP)</string>
```

- [ ] **Step 9: Compile `FleetSnapshot` and `SnapshotStore` into both targets**

In `apps/ios/generate-project.py`, add to `AGENTKIT_SOURCES`:

```python
    # In BOTH this list and `ACTIVITY_SOURCES`' companion set below, for the
    # same reason `AgentActivityAttributes.swift` is: two targets, two binaries,
    # one file. The widget renders the snapshot the app writes, and a second
    # copy of either type is two definitions of the file they share.
    "FleetSnapshot.swift",
    "SnapshotStore.swift",
```

and change `activity_build_ids` to include them:

```python
activity_build_ids = {
    name: oid("activity-build/" + name)
    for name in ACTIVITY_SOURCES
    + ["AgentActivityAttributes.swift", "FleetSnapshot.swift", "SnapshotStore.swift"]
}
```

Then find every place that iterates `ACTIVITY_SOURCES + ["AgentActivityAttributes.swift"]` to build the extension's sources phase and its group children, and extend each with the two new names. Search for the literal:

```bash
grep -n 'AgentActivityAttributes.swift"\]' apps/ios/generate-project.py
```

Every hit must list all three files.

- [ ] **Step 10: Regenerate and build**

```bash
python3 apps/ios/generate-project.py
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination "generic/platform=iOS" ARCHS=arm64 ONLY_ACTIVE_ARCH=NO \
  CODE_SIGNING_ALLOWED=NO build 2>&1 | grep -E "error:|BUILD"
```

Expected: no `error:` lines. Then confirm the group reached both targets:

```bash
grep -c "FARCOOLER_APP_GROUP = group.com.farcooler.ios" apps/ios/FarCooler.xcodeproj/project.pbxproj
```

Expected: 4 (debug and release, app and extension).

- [ ] **Step 11: Commit**

```bash
git add apps/shared/AgentKit apps/ios/FarCooler/FarCooler.entitlements \
        apps/ios/FarCoolerActivity/FarCoolerActivity.entitlements \
        apps/ios/FarCooler/Info.plist apps/ios/FarCoolerActivity/Info.plist \
        apps/ios/generate-project.py apps/ios/FarCooler.xcodeproj/project.pbxproj
git commit -m "feat(ios): a container the app and its widget both know"
```

---

### Task 4: The app writes the snapshot

**Files:**
- Modify: `apps/ios/FarCooler/Connection.swift:454` (after the fleet decodes)
- Create: `apps/ios/FarCooler/FleetSnapshotWriter.swift`

**Interfaces:**
- Consumes: `FleetSnapshot`, `SnapshotStore` (Tasks 2–3); `Terminal.glyph`/`headline`/`line`/`rank`/`feed` (Task 1).
- Produces: `FleetSnapshotWriter.write(fleet:machine:)`. Nothing later consumes it directly; Task 9 reads what it wrote.

- [ ] **Step 1: Write the writer**

Create `apps/ios/FarCooler/FleetSnapshotWriter.swift`:

```swift
import Foundation
import WidgetKit

/// Turning the fleet the app just polled into the snapshot every other surface
/// renders from.
///
/// Deliberately a projection and not a second model. Each field is copied from
/// what the daemon derived — nothing here decides what a headline says or which
/// agent ranks first, because those are decided on the host precisely so a
/// widget and this app cannot disagree about the same pane.
///
/// Only agent panes reach the snapshot. A plain shell has no `activity`, and a
/// widget listing every terminal on every machine would be a list nobody can
/// find anything in.
enum FleetSnapshotWriter {
    static func write(fleet: Fleet, machine: String) {
        let agents = fleet.workspaces.flatMap(\.terminals).compactMap { terminal in
            snapshotAgent(terminal, machine: machine)
        }
        SnapshotStore.write(
            FleetSnapshot(agents: agents, capturedAt: Date(), complete: true))
        // The surfaces are out of process and do not poll. Without this they
        // keep drawing the previous snapshot until the system next decides to
        // refresh them, which can be an hour.
        WidgetCenter.shared.reloadAllTimelines()
    }

    private static func snapshotAgent(
        _ terminal: Terminal, machine: String
    ) -> FleetSnapshot.Agent? {
        // No activity means the host is not reporting this as an agent at all.
        // A snapshot entry for it would put a shell on a lock screen.
        guard let activity = terminal.activity, !activity.isEmpty else { return nil }
        return FleetSnapshot.Agent(
            id: terminal.id,
            label: terminal.label,
            machine: machine,
            status: activity,
            glyph: terminal.glyph ?? "",
            headline: terminal.headline ?? "",
            line: terminal.signalLine,
            feed: terminal.feed ?? [],
            // A daemon too old to rank sorts last rather than first: it cannot
            // tell us this pane is urgent, and guessing that it is would put an
            // unknown above a known blocked agent.
            rank: terminal.sortRank,
            turnFailed: terminal.turnFailed ?? false,
            activityChangedAt: nil)
    }
}
```

- [ ] **Step 2: Call it when the fleet decodes**

In `apps/ios/FarCooler/Connection.swift`, immediately after line 454's `fleet = try JSONDecoder().decode(Fleet.self, from: data)`:

```swift
            // The glance surfaces render from this and cannot fetch it
            // themselves. Written on every poll rather than on change, because
            // `capturedAt` is what makes the widget able to say how old it is,
            // and a snapshot only rewritten on change would claim to be as old
            // as the last state change rather than as old as the last look.
            FleetSnapshotWriter.write(fleet: fleet, machine: machineName)
```

Use whatever this `Connection` already calls its machine — check the surrounding scope for the existing property name and use it verbatim rather than introducing one.

- [ ] **Step 3: Register the new file**

Add `"FleetSnapshotWriter.swift",` to `SOURCES` in `apps/ios/generate-project.py`, then:

```bash
python3 apps/ios/generate-project.py
```

- [ ] **Step 4: Build**

```bash
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination "generic/platform=iOS" ARCHS=arm64 ONLY_ACTIVE_ARCH=NO \
  CODE_SIGNING_ALLOWED=NO build 2>&1 | grep -E "error:|BUILD"
```

Expected: no `error:` lines.

- [ ] **Step 5: Verify the file appears on device**

Run on a device connected to a machine with agents. Then confirm the container has a snapshot — easiest from the app itself, by temporarily printing `SnapshotStore.read()?.agents.count` in a debug build, or by checking the container through Xcode's device window. Expected: a count matching the agent panes in the fleet, and `complete == true`.

- [ ] **Step 6: Commit**

```bash
git add apps/ios/FarCooler/FleetSnapshotWriter.swift \
        apps/ios/FarCooler/Connection.swift apps/ios/generate-project.py \
        apps/ios/FarCooler.xcodeproj/project.pbxproj
git commit -m "feat(ios): write down what the fleet looked like"
```

---

### Task 5: The relay carries enough for an extension to act on

The alert push carries a title, a body and a terminal. An extension that is to update the snapshot needs the status and the label too — and `mutable-content`, without which it is never invoked at all.

**Files:**
- Modify: `services/relay/src/push.ts` (the `Payload` interface and `sendApns`)
- Modify: `services/relay/src/index.ts:569-579`

**Interfaces:**
- Consumes: the `Notification` body at `services/relay/src/index.ts:503-510`, which already has optional `status` and `label`.
- Produces: an APNs alert payload carrying `terminal`, `status` and `label` at the top level, with `mutable-content: 1`. Task 6 consumes these.

- [ ] **Step 1: Widen the payload**

In `services/relay/src/push.ts`, replace the `Payload` interface:

```ts
export interface Payload {
  title: string
  subtitle: string
  /// The terminal to open. Enough to make the notification actionable and
  /// nothing more: no transcript, no command, no output. The relay is a
  /// delivery service and should not be able to leak a conversation it never
  /// held.
  terminal: string
  /// `working`, `blocked` or `done`, and the agent's name.
  ///
  /// Not for display — the title and body already say it in a sentence. These
  /// are for the phone's notification service extension, which folds them into
  /// the snapshot its widgets render so that a lock screen widget agrees with
  /// the banner that just arrived. Optional because a daemon built before them
  /// sends neither, and gets exactly the behavior it always got.
  status?: string
  label?: string
}
```

- [ ] **Step 2: Put them in the alert, and let the extension run**

In `sendApns`, replace the `body:` argument:

```ts
    body: JSON.stringify({
      aps: {
        alert: { title: payload.title, body: payload.subtitle },
        sound: 'default',
        'interruption-level': 'time-sensitive',
        'thread-id': payload.terminal,
        // Without this the notification service extension is never invoked,
        // and the phone's widgets stay on whatever the app last wrote — which
        // on a phone nobody has opened today is nothing at all. It costs the
        // extension roughly thirty milliseconds and changes nothing about how
        // the banner looks.
        'mutable-content': 1,
      },
      terminal: payload.terminal,
      status: payload.status,
      label: payload.label,
    }),
```

- [ ] **Step 3: Pass them through**

In `services/relay/src/index.ts`, replace the object at lines 573-577:

```ts
      {
        title: body.title,
        subtitle: body.subtitle ?? '',
        terminal: body.terminal ?? '',
        status: body.status,
        label: body.label,
      },
```

- [ ] **Step 4: Check it typechecks**

```bash
cd services/relay && npx tsc --noEmit && cd -
```

Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add services/relay/src/push.ts services/relay/src/index.ts
git commit -m "feat(relay): tell the phone what the push was about"
```

---

### Task 6: The notification service extension

A third target. It merges the one agent a push is about into the snapshot and reloads the widgets.

**Files:**
- Create: `apps/ios/FarCoolerNotify/NotificationService.swift`
- Create: `apps/ios/FarCoolerNotify/Info.plist`
- Create: `apps/ios/FarCoolerNotify/FarCoolerNotify.entitlements`
- Modify: `apps/ios/generate-project.py`

**Interfaces:**
- Consumes: `FleetSnapshot`, `SnapshotStore` (Tasks 2–3); the payload keys from Task 5.
- Produces: nothing other tasks consume.

**Note on scope:** this extension runs for **alert** pushes only. The `working` card updates Task 7 adds are `apns-push-type: liveactivity` and never invoke it. That is correct rather than a gap — the widget is refreshed on the state transitions that change what it says, and within-turn progress belongs to the Live Activity, which is getting it directly.

- [ ] **Step 1: Write the extension**

Create `apps/ios/FarCoolerNotify/NotificationService.swift`:

```swift
import UserNotifications
import WidgetKit

/// Keeping the widgets current when the app is not running.
///
/// A widget cannot fetch anything, and the app may not have run for hours. This
/// is the only code that touches the snapshot while the phone is in a pocket:
/// a push arrives, the one agent it is about is folded in, and the timelines
/// are reloaded.
///
/// Everything here is best-effort. The extension has roughly thirty seconds and
/// exactly one obligation — deliver the notification — so a snapshot that
/// cannot be written must not delay or drop the banner. That is why
/// `contentHandler` is called on every path, including the ones that give up.
///
/// It does NOT run for Live Activity pushes. Those are a different push type
/// and go straight to the card; see Task 7's note. The widget is therefore
/// refreshed on state CHANGES, which is exactly when what it says changes.
final class NotificationService: UNNotificationServiceExtension {
    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        // The banner is untouched. This extension exists to update a file, not
        // to rewrite what the notification says — the daemon composed that
        // sentence and it is already right.
        defer { contentHandler(request.content) }

        let info = request.content.userInfo
        guard
            let terminal = info["terminal"] as? String, !terminal.isEmpty,
            let status = info["status"] as? String, !status.isEmpty
        else { return }

        let label = info["label"] as? String ?? ""
        let now = Date()
        // Nothing on disk yet means nothing has ever polled, and a snapshot
        // built from here is partial by definition — `FleetSnapshot.empty`
        // carries `complete == false`, and `merging` keeps it false.
        let existing = SnapshotStore.read() ?? .empty
        let previous = existing.agents.first { $0.id == terminal }

        // The push carries a status and a name and nothing else. Every other
        // field is kept from what the app last wrote rather than blanked: a
        // card that lost its machine name because a push did not repeat it
        // would be a row that got worse when news arrived.
        let agent = FleetSnapshot.Agent(
            id: terminal,
            label: label.isEmpty ? (previous?.label ?? "") : label,
            machine: previous?.machine ?? "",
            status: status,
            glyph: glyph(for: status),
            headline: previous?.headline ?? "",
            line: request.content.body,
            feed: previous?.feed ?? [],
            // Ranked as the most urgent thing known. The host computes `rank`
            // from a whole fleet and this extension can see one agent, so the
            // honest projection of "this just became news" is the top — and a
            // blocked agent genuinely does outrank everything.
            rank: 0,
            turnFailed: previous?.turnFailed ?? false,
            activityChangedAt: now)

        SnapshotStore.write(existing.merging(agent, at: now))
        WidgetCenter.shared.reloadAllTimelines()
    }

    /// The same marks `farcooler_core::feed::glyph` uses.
    ///
    /// Duplicated here rather than derived, because this extension never sees a
    /// terminal — only a status word. Kept to the same five characters so a
    /// widget refreshed by a push and one refreshed by the app do not draw the
    /// same agent differently.
    private func glyph(for status: String) -> String {
        switch status {
        case "blocked": "?"
        case "done": "✓"
        case "working": "●"
        default: "·"
        }
    }
}
```

- [ ] **Step 2: Write its Info.plist**

Create `apps/ios/FarCoolerNotify/Info.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>$(EXECUTABLE_NAME)</string>
    <key>CFBundleIdentifier</key>
    <string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>$(PRODUCT_NAME)</string>
    <key>CFBundleDisplayName</key>
    <string>$(FARCOOLER_APP_NAME)</string>
    <key>CFBundlePackageType</key>
    <string>XPC!</string>
    <key>CFBundleShortVersionString</key>
    <string>$(MARKETING_VERSION)</string>
    <key>CFBundleVersion</key>
    <string>$(CURRENT_PROJECT_VERSION)</string>
    <!-- Read by `SnapshotStore`, exactly as in the app and the widget. -->
    <key>FarCoolerAppGroup</key>
    <string>$(FARCOOLER_APP_GROUP)</string>
    <!-- What makes this a notification service extension. Unlike the WidgetKit
         extension, this one DOES name a principal class: there is no bundle
         protocol to discover, and without the key the extension loads and does
         nothing. -->
    <key>NSExtension</key>
    <dict>
        <key>NSExtensionPointIdentifier</key>
        <string>com.apple.usernotifications.service</string>
        <key>NSExtensionPrincipalClass</key>
        <string>$(PRODUCT_MODULE_NAME).NotificationService</string>
    </dict>
</dict>
</plist>
```

- [ ] **Step 3: Write its entitlements**

Create `apps/ios/FarCoolerNotify/FarCoolerNotify.entitlements`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <!-- This extension is the only writer of the snapshot other than the app
         itself. Same group, same file, same channel. -->
    <key>com.apple.security.application-groups</key>
    <array>
        <string>$(FARCOOLER_APP_GROUP)</string>
    </array>
</dict>
</plist>
```

- [ ] **Step 4: Add the target to the generator**

In `apps/ios/generate-project.py`, mirror every construct the activity extension already has. Add to `KEYS`:

```python
    # The notification service extension: the same nine objects the activity
    # extension needs, for the same reasons. It is a second .appex embedded in
    # the same Embed Foundation Extensions phase.
    "notifyTarget", "notifyProduct", "notifyGroup", "notifySourcesPhase",
    "notifyConfigList", "notifyDebug", "notifyRelease",
    "notifyDependency", "notifyProxy",
```

Add its sources list beside `ACTIVITY_SOURCES`:

```python
# The notification service extension's own sources, in `apps/ios/FarCoolerNotify/`.
#
# A third target because a service extension is a separate process that iOS
# starts for an incoming push whether or not the app is running — which is the
# only moment the snapshot can be refreshed on a phone in a pocket.
NOTIFY_SOURCES = ["NotificationService.swift"]
```

Its build ids, on the same reasoning as `activity_build_ids`:

```python
notify_build_ids = {
    name: oid("notify-build/" + name)
    for name in NOTIFY_SOURCES + ["FleetSnapshot.swift", "SnapshotStore.swift"]
}
```

Its settings block, beside `ACTIVITY_COMMON`:

```python
NOTIFY_COMMON = f"""\t\t\t\tMARKETING_VERSION = {version("marketing")};
\t\t\t\tCURRENT_PROJECT_VERSION = {version("build")};
\t\t\t\tFARCOOLER_APP_NAME = "{version("app-name-short")}";
\t\t\t\tFARCOOLER_APP_GROUP = {APP_GROUP};
\t\t\t\tPRODUCT_NAME = FarCoolerNotify;
\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = {BUNDLE_ID}.notify;
\t\t\t\tINFOPLIST_FILE = FarCoolerNotify/Info.plist;
\t\t\t\tCODE_SIGN_ENTITLEMENTS = FarCoolerNotify/FarCoolerNotify.entitlements;
\t\t\t\tCODE_SIGN_STYLE = Manual;
\t\t\t\tCODE_SIGN_IDENTITY = "-";
\t\t\t\tCODE_SIGNING_ALLOWED = YES;
\t\t\t\tCODE_SIGNING_REQUIRED = NO;
\t\t\t\tSKIP_INSTALL = YES;"""
```

Then add, following each activity construct as the template: the `PBXFileReference` for `FarCoolerNotify.appex`, its `PBXGroup` with `path = FarCoolerNotify`, its `PBXSourcesBuildPhase`, its `PBXNativeTarget` with `productType = "com.apple.product-type.app-extension"`, its `XCConfigurationList` and two `XCBuildConfiguration`s, its `PBXTargetDependency` and `PBXContainerItemProxy`, its entry in the project's `targets` list and in `productsGroup`, and **a second `PBXBuildFile` in the existing Embed Foundation Extensions phase** — the app embeds two extensions now, not one.

- [ ] **Step 5: Regenerate and build**

```bash
python3 apps/ios/generate-project.py
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination "generic/platform=iOS" ARCHS=arm64 ONLY_ACTIVE_ARCH=NO \
  CODE_SIGNING_ALLOWED=NO build 2>&1 | grep -E "error:|BUILD"
```

Expected: no `error:` lines. Then confirm both extensions embed:

```bash
grep -c "in Embed Foundation Extensions" apps/ios/FarCooler.xcodeproj/project.pbxproj
```

Expected: at least 2 build-file entries.

- [ ] **Step 6: Verify on device**

Install on hardware per `docs/releasing.md`. Block an agent on a paired machine so a real push arrives while the app is backgrounded. Expected: the banner appears unchanged, and the widget (once Task 9 lands) or a debug read of `SnapshotStore.read()` shows that agent at `status == "blocked"`. Before Task 9, verify by reopening the app and reading the snapshot.

- [ ] **Step 7: Commit**

```bash
git add apps/ios/FarCoolerNotify apps/ios/generate-project.py \
        apps/ios/FarCooler.xcodeproj/project.pbxproj
git commit -m "feat(ios): refresh the snapshot from a push, with the app asleep"
```

---

### Task 7: A card for the whole run

Today `notification()` returns `None` for working and the relay rejects any status that is not `blocked` or `done`, so the Dynamic Island is empty during the only period there is anything to watch.

**Files:**
- Modify: `crates/daemon/src/watch.rs` (`notification`, `Observed`, the announce path)
- Modify: `services/relay/src/index.ts` (the status guard, and priority)
- Modify: `services/relay/src/push.ts` (`sendLiveActivity` priority)

**Interfaces:**
- Consumes: `farcooler_core::feed::line` via the `line` already on the announced `wire::Terminal` (`crates/daemon/src/wire.rs:258`, `apply_rungs`).
- Produces: a `/v1/notify` call with `status: "working"` and `subtitle` set to the signal line, at most one per terminal per 10 seconds. Task 8 renders it.

- [ ] **Step 1: Write the failing daemon tests**

Append to the test module in `crates/daemon/src/watch.rs`:

```rust
    /// A working agent gets a CARD, and never a banner.
    ///
    /// The rule that a working agent must not buzz is about alerts, and it
    /// stands. A live card is not an alert: it updates silently, and the whole
    /// point of it is watching a run you already know about.
    #[test]
    fn a_working_agent_gets_a_card_that_says_where_it_is() {
        let notice = notification(
            AgentActivity::Working, "claude", Some("3/7 · Designing test matrix"), false)
            .expect("working now produces a notice");
        assert_eq!(notice.status, "working");
        assert_eq!(notice.subtitle, "3/7 · Designing test matrix");
    }

    /// A tier change is never withheld. Those are the pushes the feature is for.
    #[test]
    fn a_state_change_is_never_throttled() {
        assert!(should_refresh_card(None, 1_000));
        assert!(should_refresh_card(Some(1_000), 1_000 + CARD_REFRESH_MS));
    }

    /// A line that changes three times a second is not three pushes.
    #[test]
    fn a_line_that_moves_faster_than_it_reads_is_coalesced() {
        assert!(!should_refresh_card(Some(1_000), 1_100));
        assert!(!should_refresh_card(Some(1_000), 1_000 + CARD_REFRESH_MS - 1));
    }
```

- [ ] **Step 2: Run them to verify they fail**

```bash
PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-daemon a_working_agent_gets_a_card 2>&1 | tail -20
```

Expected: FAIL — `should_refresh_card` not found, and the working assertion failing on `None`.

- [ ] **Step 3: Add the working arm and the throttle**

In `crates/daemon/src/watch.rs`, add the working arm to `notification`, before the `_ => None`:

```rust
        // A card, not a banner. `push_notice`'s caller decides which of the two
        // this becomes — the relay sends an alert for `blocked` and `done` and
        // only updates the card for `working` — so producing a notice here does
        // NOT reintroduce buzzing for the normal case. What it buys is a
        // Dynamic Island that says something during the only period there is
        // anything to watch.
        //
        // `title` is the label alone. The relay never builds an alert from a
        // working notice, so there is no sentence for it to be the subject of.
        AgentActivity::Working => Some(Notice {
            title: label.to_string(),
            subtitle: question.unwrap_or("").to_string(),
            status: "working",
        }),
```

Add the throttle beside it:

```rust
/// How often a live card may be refreshed while an agent stays in one tier.
///
/// A signal line can change several times a second, and pushing each one spends
/// the app's Live Activity budget on text nobody can read at that rate. Ten
/// seconds is roughly the fastest a changing line is still worth reading rather
/// than watching flicker, and it caps a six-agent fleet at 36 pushes a minute.
///
/// A starting number, not a measured one — see the design's risk 2.
const CARD_REFRESH_MS: i64 = 10_000;

/// Whether a within-tier card refresh is due.
///
/// Only ever asked about `working`. A tier change — working to blocked to done
/// — does not come through here at all: those are the pushes this whole feature
/// exists to deliver, and withholding one to save a byte of budget would be
/// throttling the alarm to keep the clock quiet.
fn should_refresh_card(last_push_ms: Option<i64>, now_ms: i64) -> bool {
    match last_push_ms {
        None => true,
        Some(at) => now_ms.saturating_sub(at) >= CARD_REFRESH_MS,
    }
}
```

Add the field to `Observed`, after `blocked_question`:

```rust
    /// When a live card was last refreshed for this terminal, in Unix
    /// milliseconds. `None` until the first one.
    ///
    /// Only within-tier refreshes consult it; see `should_refresh_card`.
    last_card_push: Option<i64>,
```

Initialize it to `None` everywhere `Observed` is constructed — the compiler will name each site.

- [ ] **Step 4: Push a working card from the announce path**

At the announce site where `apply_rungs` has already run and the `wire::Terminal` carries `line`, add — after the existing `push_if_paired` call for tier changes:

```rust
        // A within-tier refresh: the agent is still working and its line moved.
        // Guarded so a busy agent does not become a push per screen update.
        if activity == AgentActivity::Working && !message.line.is_empty() {
            let now = crate::review::now_millis();
            if should_refresh_card(observed.last_card_push, now) {
                observed.last_card_push = Some(now);
                self.push_if_paired(
                    terminal, AgentActivity::Working, label, Some(&message.line), false);
            }
        }
```

Set `last_card_push` on tier changes too, so a transition resets the window rather than letting a refresh fire immediately after one.

- [ ] **Step 5: Run the daemon tests**

```bash
PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-daemon 2>&1 | tail -20
```

Expected: PASS, including the three new tests. Existing tests asserting a working agent produces no notice will now fail — those assertions at `crates/daemon/src/watch.rs:3386` and `:3391` encoded the old rule and must be updated to assert the new one: working produces a notice whose `status` is `"working"`.

- [ ] **Step 6: Let the relay accept a working card**

In `services/relay/src/index.ts`, replace the guard at line 632:

```ts
  const status = body.status
  if (status !== 'blocked' && status !== 'done' && status !== 'working') return
```

and make a working push update only, never start and never alert:

```ts
  // A working card is only ever an UPDATE. Starting one from `working` would
  // put a card on the lock screen for an agent nobody asked to watch, and the
  // push-to-start token is spent on the transition that matters.
  if (status === 'working') {
    if (!running) return
    await deliverActivity(env, daemon.account_id, running.update_token, running.environment, {
      event: 'update',
      state,
      alert,
    })
    return
  }
```

Place it immediately after `running` is read and before the `status === 'done'` branch.

Then, in the alert loop at line 568, skip the banner for working:

```ts
  // A working state is a card update and nothing else. The rule that a working
  // agent must not buzz is unchanged; only the card is new.
  if (body.status !== 'working') {
    for (const device of devices.results ?? []) { /* … existing loop … */ }
  }
```

- [ ] **Step 7: Split the activity push priority**

In `services/relay/src/push.ts`, in `sendLiveActivity`, replace the hardcoded priority:

```ts
      // Priority 10 consumes the app's Live Activity push budget, and the
      // budget it consumes is the one the ALERT depends on. Routine progress
      // therefore goes out at 5: delivered when convenient, which is the right
      // urgency for a line that will change again in ten seconds.
      'apns-priority': activity.state.status === 'working' ? '5' : '10',
```

- [ ] **Step 8: Typecheck the relay**

```bash
cd services/relay && npx tsc --noEmit && cd -
```

Expected: no output.

- [ ] **Step 9: Commit**

```bash
git add crates/daemon/src/watch.rs services/relay/src/index.ts services/relay/src/push.ts
git commit -m "feat: a card that follows the whole run, not just the stuck part"
```

---

### Task 8: The card says where the agent is, and opens the right app

**Files:**
- Modify: `apps/shared/AgentKit/Sources/AgentKit/AgentActivityAttributes.swift`
- Modify: `apps/ios/FarCoolerActivity/AgentActivityWidget.swift`
- Modify: `apps/ios/FarCooler/FarCoolerApp.swift`

**Interfaces:**
- Consumes: `AgentActivityAttributes` as it stands; `FARCOOLER_URL_SCHEME` in `ACTIVITY_COMMON` from Task 3 Step 5.
- Produces: `AgentActivityAttributes.startedAt: Date?`, `ContentState.line: String`; `AppScheme.current: String`. Task 9 uses `AppScheme.current`.

- [ ] **Step 1: Add the timer's date and the signal line**

In `apps/shared/AgentKit/Sources/AgentKit/AgentActivityAttributes.swift`, add to `ContentState` after `detail`:

```swift
            /// Where the agent is, in one line, as the host composed it.
            ///
            /// Separate from `detail` rather than replacing it: `detail` is
            /// what the notification's body said, and the two differ for a
            /// working card, which has no notification under it at all.
            ///
            /// Defaulted on decode so a card started by an older relay — one
            /// that sends no `line` — updates rather than freezing on whatever
            /// it last showed, which is what a decode failure looks like from
            /// the lock screen.
            public var line: String = ""
```

and add to the attributes, after `machine`:

```swift
    /// When the turn started, so the card can show its own clock.
    ///
    /// On the ATTRIBUTES rather than the state, because it does not change for
    /// the life of the card and because ActivityKit renders a timer from a date
    /// natively — no push per tick, and it keeps counting while the phone is
    /// off the network entirely.
    ///
    /// Optional: a card started for an agent whose turn clock the host could
    /// not read shows no timer, which is better than showing a wrong one.
    public var startedAt: Date?
```

Update the memberwise `init` to take `startedAt: Date? = nil` — defaulted so the relay's existing start payload, which sends no such key, still decodes.

- [ ] **Step 2: Render the line and the timer**

In `apps/ios/FarCoolerActivity/AgentActivityWidget.swift`, in `LockScreenCard`, replace the `if !state.detail.isEmpty` block with:

```swift
                // The signal line when the host composed one, else whatever the
                // notification said. A working card has no notification under
                // it, so `detail` is empty for exactly the case `line` covers.
                let body = state.line.isEmpty ? state.detail : state.line
                if !body.isEmpty {
                    Text(body)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .padding(.top, 2)
                }
                if let started = attributes.startedAt {
                    Text(started, style: .timer)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.tertiary)
                }
```

Make the same substitution in the `DynamicIslandExpandedRegion(.bottom)` block.

- [ ] **Step 3: Fix the deep link**

Add to `apps/ios/FarCoolerActivity/AgentActivityWidget.swift`:

```swift
/// This build's URL scheme.
///
/// `farcooler` for stable, `farcooler-canary` and friends for the rest. It has
/// to be read rather than written down: this file hardcoded `farcooler://`,
/// which every non-stable channel does not register — so tapping a canary
/// card opened STABLE if it was installed and nothing at all if it was not.
/// The app's own Info.plist documents the same hazard for sign-in; the widget
/// was missed because `ACTIVITY_COMMON` does not inherit `TARGET_COMMON`.
enum AppScheme {
    static var current: String {
        Bundle.main.object(forInfoDictionaryKey: "FarCoolerURLScheme") as? String
            ?? "farcooler"
    }
}
```

Add the key to `apps/ios/FarCoolerActivity/Info.plist`:

```xml
    <key>FarCoolerURLScheme</key>
    <string>$(FARCOOLER_URL_SCHEME)</string>
```

Replace line 73:

```swift
            .widgetURL(URL(string: "\(AppScheme.current)://terminal/\(context.attributes.terminal)"))
```

- [ ] **Step 4: Route the URL in the app**

In `apps/ios/FarCooler/FarCoolerApp.swift`, add to the root view:

```swift
        .onOpenURL { url in
            // `…://terminal/<id>`. The scheme is this channel's and is not
            // checked here — iOS only delivers URLs whose scheme this app
            // registered, so a canary build cannot be handed a stable link.
            guard url.host == "terminal" else { return }
            let terminal = url.lastPathComponent
            guard !terminal.isEmpty else { return }
            connection.select(terminal: terminal)
        }
```

Use whatever this app already calls "show this terminal" — check `FleetView`'s `onSelect` path and reuse that call rather than adding a second way to select.

- [ ] **Step 5: Build**

```bash
python3 apps/ios/generate-project.py
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination "generic/platform=iOS" ARCHS=arm64 ONLY_ACTIVE_ARCH=NO \
  CODE_SIGNING_ALLOWED=NO build 2>&1 | grep -E "error:|BUILD"
```

Expected: no `error:` lines.

- [ ] **Step 6: Verify on device**

Install a **canary** build (`FARCOOLER_CHANNEL=canary`) alongside stable if you have both. Block an agent. Expected: a card appears, its line reads what the agent is asking, a timer counts, and tapping it opens **canary** rather than stable. This is the one verification that cannot be done on a single-channel install.

- [ ] **Step 7: Commit**

```bash
git add apps/shared/AgentKit/Sources/AgentKit/AgentActivityAttributes.swift \
        apps/ios/FarCoolerActivity apps/ios/FarCooler/FarCoolerApp.swift \
        apps/ios/FarCooler.xcodeproj/project.pbxproj
git commit -m "fix(ios): a card that counts, says where, and opens its own app"
```

---

### Task 9: The widgets

**Files:**
- Create: `apps/ios/FarCoolerActivity/FleetWidget.swift`
- Modify: `apps/ios/FarCoolerActivity/FarCoolerActivityBundle.swift`
- Modify: `apps/ios/generate-project.py` (`ACTIVITY_SOURCES`)

**Interfaces:**
- Consumes: `FleetSnapshot`, `SnapshotStore` (Tasks 2–3), `AppScheme.current` (Task 8).
- Produces: the finished surfaces. Nothing consumes them.

- [ ] **Step 1: Write the widget**

Create `apps/ios/FarCoolerActivity/FleetWidget.swift`:

```swift
import SwiftUI
import WidgetKit

/// The fleet, on a home screen and under a lock screen clock.
///
/// Everything drawn here was decided on the host. `glyph`, `headline`, `line`
/// and the order come off the snapshot unchanged — this file chooses which rung
/// fits the space and nothing else, which is the entire reason the ladder is
/// computed once in `farcooler_core::feed` rather than by each client.
///
/// The one judgement it does make is how confident to sound. A snapshot is
/// always somewhat old, and `FleetSnapshot.confidence(in:at:)` answers whether
/// a given agent's status has stopped being trustworthy — blocked and done stay
/// true, working does not.
struct FleetWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "FleetWidget", provider: FleetProvider()) { entry in
            FleetWidgetView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Agents")
        .description("What your agents are doing, and which one needs you.")
        .supportedFamilies([
            .systemSmall, .systemMedium, .systemLarge,
            .accessoryCircular, .accessoryRectangular, .accessoryInline,
        ])
    }
}

struct FleetEntry: TimelineEntry {
    let date: Date
    let snapshot: FleetSnapshot
}

struct FleetProvider: TimelineProvider {
    func placeholder(in context: Context) -> FleetEntry {
        FleetEntry(date: Date(), snapshot: .empty)
    }

    func getSnapshot(in context: Context, completion: @escaping (FleetEntry) -> Void) {
        completion(FleetEntry(date: Date(), snapshot: SnapshotStore.read() ?? .empty))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<FleetEntry>) -> Void) {
        let snapshot = SnapshotStore.read() ?? .empty
        let now = Date()

        // Two entries, and the second one is the point.
        //
        // Refreshes arrive from outside — the app writes and reloads, the
        // notification service extension writes and reloads — so one entry
        // would be enough to show what is known. But a widget that can only
        // learn it has gone stale from a message it is not receiving would
        // assert `working` forever. The second entry is the moment this
        // snapshot stops being trustworthy, scheduled so the widget can say so
        // on its own.
        let goesStale = snapshot.capturedAt.addingTimeInterval(FleetSnapshot.staleAfter)
        let entries =
            goesStale > now
            ? [FleetEntry(date: now, snapshot: snapshot),
               FleetEntry(date: goesStale, snapshot: snapshot)]
            : [FleetEntry(date: now, snapshot: snapshot)]

        completion(Timeline(entries: entries, policy: .never))
    }
}

struct FleetWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: FleetEntry

    private var agents: [FleetSnapshot.Agent] { entry.snapshot.ranked }
    private var top: FleetSnapshot.Agent? { agents.first }

    var body: some View {
        switch family {
        case .accessoryInline:
            Text(top?.headline ?? "No agents")
        case .accessoryCircular:
            VStack(spacing: 0) {
                Text(top?.glyph ?? "·").font(.title3)
                Text("\(entry.snapshot.needingYou)").font(.caption2.monospacedDigit())
            }
        case .accessoryRectangular:
            VStack(alignment: .leading, spacing: 1) {
                Text(top?.headline ?? "No agents").font(.headline)
                if let top, !top.line.isEmpty {
                    Text(top.line).font(.caption).lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case .systemSmall:
            SmallFleet(entry: entry)
        case .systemLarge:
            RowsFleet(entry: entry, limit: 6, showFeed: true)
        default:
            RowsFleet(entry: entry, limit: 3, showFeed: false)
        }
    }
}

/// One number and the agent behind it.
private struct SmallFleet: View {
    let entry: FleetEntry

    var body: some View {
        let waiting = entry.snapshot.needingYou
        VStack(alignment: .leading, spacing: 6) {
            Text("\(waiting)")
                .font(.system(size: 44, weight: .semibold, design: .rounded))
                .foregroundStyle(waiting > 0 ? .orange : .secondary)
            Text(waiting == 1 ? "agent needs you" : "agents need you")
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
            if let top = entry.snapshot.ranked.first {
                AgentLine(agent: top, snapshot: entry.snapshot, at: entry.date)
            }
            StaleFooter(entry: entry)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A list of agents, most urgent first.
private struct RowsFleet: View {
    let entry: FleetEntry
    let limit: Int
    let showFeed: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if entry.snapshot.agents.isEmpty {
                Text("No agents").font(.headline).foregroundStyle(.secondary)
            }
            ForEach(entry.snapshot.ranked.prefix(limit)) { agent in
                Link(destination: URL(string: "\(AppScheme.current)://terminal/\(agent.id)")!) {
                    AgentLine(agent: agent, snapshot: entry.snapshot, at: entry.date)
                }
            }
            // The large family's whole reason for being: three lines of what
            // the agent actually said answers "what did it do while I was
            // away", which is the question a widget gets looked at to answer.
            if showFeed, let top = entry.snapshot.ranked.first, !top.feed.isEmpty {
                Divider()
                ForEach(Array(top.feed.enumerated()), id: \.offset) { _, said in
                    Text(said).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            StaleFooter(entry: entry)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// One agent: its mark, its name, and where it is.
private struct AgentLine: View {
    let agent: FleetSnapshot.Agent
    let snapshot: FleetSnapshot
    let at: Date

    var body: some View {
        let confidence = snapshot.confidence(in: agent, at: at)
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(agent.glyph)
                .font(.caption.monospaced())
                .foregroundStyle(agent.status == "blocked" ? .orange : .secondary)
            VStack(alignment: .leading, spacing: 0) {
                // "last seen working" rather than "working": an agent that was
                // working an hour ago has very likely finished, and a widget
                // that keeps asserting it is a widget telling you something
                // untrue in the calmest possible voice.
                Text(confidence == .lastSeen ? "last seen \(agent.headline)" : agent.headline)
                    .font(.caption.weight(.medium))
                    .lineLimit(1)
                if !agent.line.isEmpty {
                    Text(agent.line)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .opacity(confidence == .lastSeen ? 0.6 : 1)
    }
}

/// How old this is, and whether it is the whole fleet.
private struct StaleFooter: View {
    let entry: FleetEntry

    var body: some View {
        // A snapshot assembled only from pushes knows about the agents that
        // happened to notify. Saying so is the difference between "these are
        // your agents" and "these are the ones I have heard from".
        let source = entry.snapshot.complete ? "" : " · from notifications"
        if entry.snapshot.capturedAt.timeIntervalSince1970 > 0 {
            Text("\(entry.snapshot.capturedAt, style: .relative) ago\(source)")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        } else {
            Text("Open Far Cooler to see your agents")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
    }
}
```

- [ ] **Step 2: Register it**

Replace the body of `apps/ios/FarCoolerActivity/FarCoolerActivityBundle.swift`:

```swift
    var body: some Widget {
        AgentActivityWidget()
        FleetWidget()
    }
```

- [ ] **Step 3: Add it to the generator**

In `apps/ios/generate-project.py`, add `"FleetWidget.swift",` to `ACTIVITY_SOURCES`, then:

```bash
python3 apps/ios/generate-project.py
```

- [ ] **Step 4: Build**

```bash
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination "generic/platform=iOS" ARCHS=arm64 ONLY_ACTIVE_ARCH=NO \
  CODE_SIGNING_ALLOWED=NO build 2>&1 | grep -E "error:|BUILD"
```

Expected: no `error:` lines.

- [ ] **Step 5: Verify every family on device**

Install on hardware. Add the widget in all three home screen sizes and all three lock screen families. Check, in order:

1. With agents running: rows appear, most urgent first, and the blocked one is at the top with an orange mark.
2. Tapping a row opens that terminal, not the app's front door.
3. Large shows the top agent's last three lines.
4. Force-quit the app, block an agent from the machine: the widget updates from the push alone.
5. On a phone that has never opened the app: the footer reads "Open Far Cooler to see your agents" rather than an empty list that looks like an empty fleet.

- [ ] **Step 6: Commit**

```bash
git add apps/ios/FarCoolerActivity apps/ios/generate-project.py \
        apps/ios/FarCooler.xcodeproj/project.pbxproj
git commit -m "feat(ios): the fleet on a home screen and under a lock screen clock"
```

---

## Self-Review

**Spec coverage.** Every section of the design maps to a task: Task 0 is the spike; Task 1 is "the iOS app catches up to the ladder"; Tasks 2–3 are the snapshot, the per-channel App Group and the staleness rules; Task 4 is the app as a writer; Tasks 5–6 are the NSE and the relay payload it needs; Task 7 is the whole-run card and tier throttling; Task 8 is elapsed time, the signal line and the scheme fix; Task 9 is the widgets, including the scheduled staleness entry and the `complete` footer.

**One spec item deliberately not implemented:** in-place Allow / Deny via `LiveActivityIntent`, which the design defers pending a measured cold connect. Task 8 ships the deep link instead, as specified.

**Known ordering constraint:** Task 7 changes the relay's accepted statuses and the daemon's push behavior. Deploy the relay before the daemon, or a `working` notify is rejected — harmlessly, but it will look like the throttle is broken.
