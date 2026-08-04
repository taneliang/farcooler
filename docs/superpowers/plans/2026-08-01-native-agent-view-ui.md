# Native Agent View — Client UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render an ACP agent session natively — as a tile beside terminals on macOS and as a full-screen view on iOS — against the contract settled by the core plan.

**Architecture:** One shared Swift package decodes the daemon's normalized `AgentEvent` JSON and reduces it into a renderable transcript. Each platform writes only views. On macOS an agent pane draws into the rectangle tmux already assigned it, so tiling, dividers and focus navigation need no new code.

**Tech Stack:** Swift 6 / SwiftUI, SwiftPM for macOS, a generated Xcode project for iOS, protobuf over the existing daemon client.

**Source spec:** `docs/superpowers/specs/2026-08-01-native-agent-view-design.md` (slices 6–8).
**Predecessor:** `docs/superpowers/plans/2026-08-01-native-agent-view-core.md` (slices 1–5), which must be complete first.

## Global Constraints

- Three distinct names, never the bare word alone: `mode` = VT modes; `paneMode` = TERMINAL vs AGENT; `agentMode` = the ACP concept.
- A `Gap` event MUST render as a visible break in the transcript. Never omit it, never collapse it into adjacent messages. This is the contract that lets a derived transcript exist at all.
- Clients never derive state the daemon owns. `paneMode`, `activity`, `agentMode` are rendered as received.
- No new permanent chrome on a pane. `TerminalPane` deliberately has no header or footer; the agent surface must not reintroduce one. Mode and attachments live in the composer row; the terminal↔chat toggle lives with the other pane commands (`⌃B` binding, command palette, context menu).
- macOS deployment target `.macOS(.v14)`, Swift tools 6.0, matching `apps/macos/Package.swift`.
- No syntax highlighting inside diffs. Cut in the spec.

---

### Task 1: The shared `AgentKit` package

**Files:**
- Create: `apps/shared/AgentKit/Package.swift`
- Create: `apps/shared/AgentKit/Sources/AgentKit/AgentEvent.swift`
- Create: `apps/shared/AgentKit/Tests/AgentKitTests/AgentEventTests.swift`
- Modify: `apps/macos/Package.swift`

**Interfaces:**
- Produces: `AgentKit.AgentEvent` (enum), `AgentKit.Sequenced`, `AgentKit.Role`, `AgentKit.ToolStatus`, `AgentKit.Diff`, `AgentKit.PlanEntry`, `AgentKit.PermissionOption`, `AgentKit.GapReason`.

The Rust side serializes `farcooler_agent::event::AgentEvent` with serde's default externally-tagged representation. These are the **measured** outputs, printed from a scratch test against the real types — not a guess, and the decoder must match them exactly:

```json
{"Message":{"role":"Agent","text":"hi"}}
{"Gap":{"reason":"RingTrimmed"}}
{"TurnEnded":{"reason":"EndTurn"}}
{"ToolUpdate":{"id":"t","status":"InProgress","content":null,"diff":{"path":"p","old_text":"o","new_text":"n"}}}
{"SessionStarted":{"session_id":"s","agent_mode":"default","available_modes":["plan"],"available_commands":[]}}
{"Permission":{"id":"r","tool_call":"t","options":[{"id":"a","name":"Allow","kind":"allow_once"}]}}
```

Note: variant names and unit-enum values are `PascalCase`; struct fields are `snake_case`. A wrong assumption here fails at runtime, not at compile time.

- [ ] **Step 1: Write the failing test**

Create `apps/shared/AgentKit/Tests/AgentKitTests/AgentEventTests.swift`:

```swift
import Testing
@testable import AgentKit

@Test func anAgentMessageDecodes() throws {
    let json = #"{"Message":{"role":"Agent","text":"hello"}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .message(role, text) = event else {
        Issue.record("expected a message, got \(event)")
        return
    }
    #expect(role == .agent)
    #expect(text == "hello")
}

@Test func aGapDecodesAsAGapAndNotAsNothing() throws {
    // The contract the whole feature rests on. A decoder that silently
    // skipped an event it did not understand would produce a transcript that
    // is wrong and looks complete.
    let json = #"{"Gap":{"reason":"RingTrimmed"}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .gap(reason) = event else {
        Issue.record("expected a gap, got \(event)")
        return
    }
    #expect(reason == .ringTrimmed)
}

@Test func anEventFromALaterDaemonBecomesAGapRatherThanAThrow() throws {
    // A client one release behind its daemon must still render the session.
    // Throwing would blank the whole transcript over one unknown event;
    // dropping it would silently shorten it. A gap is the honest third option.
    let json = #"{"SomethingInvented":{"x":1}}"#
    let event = try AgentEvent.decode(from: json)
    #expect(event == .gap(.unparsed))
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/shared/AgentKit && swift test`
Expected: FAIL — no such package / `AgentEvent` undefined

- [ ] **Step 3: Create the package and the decoder**

`apps/shared/AgentKit/Package.swift`:

```swift
// swift-tools-version: 6.0
import PackageDescription

// Shared because both apps must agree about one session.
//
// The two apps already duplicate Model.swift and VTCore.swift by copy. This
// body of logic is larger than either, and two copies would drift in exactly
// the way that makes a phone and a Mac disagree about the same terminal —
// which is the failure the whole derivation model exists to prevent.
let package = Package(
    name: "AgentKit",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "AgentKit", targets: ["AgentKit"])],
    targets: [
        .target(name: "AgentKit"),
        .testTarget(name: "AgentKitTests", dependencies: ["AgentKit"]),
    ]
)
```

`apps/shared/AgentKit/Sources/AgentKit/AgentEvent.swift`:

```swift
import Foundation

public enum Role: String, Decodable, Sendable {
    case user = "User"
    case agent = "Agent"
    case thought = "Thought"
}

public enum ToolStatus: String, Decodable, Sendable {
    case pending = "Pending"
    case inProgress = "InProgress"
    case completed = "Completed"
    case failed = "Failed"
}

public enum GapReason: String, Decodable, Sendable {
    case ringTrimmed = "RingTrimmed"
    case loadUnsupported = "LoadUnsupported"
    case unparsed = "Unparsed"
}

public struct Diff: Decodable, Sendable, Equatable {
    public let path: String
    public let oldText: String?
    public let newText: String

    enum CodingKeys: String, CodingKey {
        case path
        case oldText = "old_text"
        case newText = "new_text"
    }
}

public struct PlanEntry: Decodable, Sendable, Equatable {
    public let content: String
    public let priority: String
    public let status: String
}

public struct PermissionOption: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let kind: String
}

public enum AgentEvent: Sendable, Equatable {
    case sessionStarted(sessionID: String, agentMode: String?, availableModes: [String], availableCommands: [String])
    case message(role: Role, text: String)
    case toolCall(id: String, title: String, kind: String, status: ToolStatus, locations: [String])
    case toolUpdate(id: String, status: ToolStatus, content: String?, diff: Diff?)
    case plan(entries: [PlanEntry])
    case permission(id: String, toolCall: String, options: [PermissionOption])
    case resolved(id: String, chosen: String)
    case modeSet(agentMode: String)
    case turnEnded(reason: String)
    case gap(GapReason)
}

public struct Sequenced: Sendable, Equatable {
    public let seq: UInt64
    public let event: AgentEvent

    public init(seq: UInt64, event: AgentEvent) {
        self.seq = seq
        self.event = event
    }
}

extension AgentEvent {
    /// Decode one serialized `farcooler_agent::event::AgentEvent`.
    ///
    /// Serde's externally-tagged representation: a single-key object whose key
    /// names the variant. An unrecognized key is a `.gap(.unparsed)` rather
    /// than a throw, because a client one release behind its daemon must still
    /// render the session — and rather than a silent skip, because a shorter
    /// transcript that looks complete is the one failure this design refuses.
    public static func decode(from json: String) throws -> AgentEvent {
        guard let data = json.data(using: .utf8) else { return .gap(.unparsed) }
        return try decode(from: data)
    }

    public static func decode(from data: Data) throws -> AgentEvent {
        let container = try JSONDecoder().decode(Envelope.self, from: data)
        return container.event
    }

    private struct Envelope: Decodable {
        let event: AgentEvent

        private struct Key: CodingKey {
            var stringValue: String
            var intValue: Int? { nil }
            init?(stringValue: String) { self.stringValue = stringValue }
            init?(intValue: Int) { nil }
        }

        init(from decoder: Decoder) throws {
            let outer = try decoder.container(keyedBy: Key.self)
            guard let key = outer.allKeys.first else {
                self.event = .gap(.unparsed)
                return
            }
            switch key.stringValue {
            case "SessionStarted":
                let p = try outer.decode(SessionStartedPayload.self, forKey: key)
                event = .sessionStarted(
                    sessionID: p.sessionID, agentMode: p.agentMode,
                    availableModes: p.availableModes, availableCommands: p.availableCommands)
            case "Message":
                let p = try outer.decode(MessagePayload.self, forKey: key)
                event = .message(role: p.role, text: p.text)
            case "ToolCall":
                let p = try outer.decode(ToolCallPayload.self, forKey: key)
                event = .toolCall(
                    id: p.id, title: p.title, kind: p.kind, status: p.status,
                    locations: p.locations)
            case "ToolUpdate":
                let p = try outer.decode(ToolUpdatePayload.self, forKey: key)
                event = .toolUpdate(id: p.id, status: p.status, content: p.content, diff: p.diff)
            case "Plan":
                let p = try outer.decode(PlanPayload.self, forKey: key)
                event = .plan(entries: p.entries)
            case "Permission":
                let p = try outer.decode(PermissionPayload.self, forKey: key)
                event = .permission(id: p.id, toolCall: p.toolCall, options: p.options)
            case "Resolved":
                let p = try outer.decode(ResolvedPayload.self, forKey: key)
                event = .resolved(id: p.id, chosen: p.chosen)
            case "ModeSet":
                let p = try outer.decode(ModeSetPayload.self, forKey: key)
                event = .modeSet(agentMode: p.agentMode)
            case "TurnEnded":
                let p = try outer.decode(TurnEndedPayload.self, forKey: key)
                event = .turnEnded(reason: p.reason)
            case "Gap":
                let p = try outer.decode(GapPayload.self, forKey: key)
                event = .gap(p.reason)
            default:
                event = .gap(.unparsed)
            }
        }
    }

    private struct SessionStartedPayload: Decodable {
        let sessionID: String
        let agentMode: String?
        let availableModes: [String]
        let availableCommands: [String]
        enum CodingKeys: String, CodingKey {
            case sessionID = "session_id"
            case agentMode = "agent_mode"
            case availableModes = "available_modes"
            case availableCommands = "available_commands"
        }
    }
    private struct MessagePayload: Decodable { let role: Role; let text: String }
    private struct ToolCallPayload: Decodable {
        let id: String; let title: String; let kind: String
        let status: ToolStatus; let locations: [String]
    }
    private struct ToolUpdatePayload: Decodable {
        let id: String; let status: ToolStatus; let content: String?; let diff: Diff?
    }
    private struct PlanPayload: Decodable { let entries: [PlanEntry] }
    private struct PermissionPayload: Decodable {
        let id: String; let toolCall: String; let options: [PermissionOption]
        enum CodingKeys: String, CodingKey { case id, options, toolCall = "tool_call" }
    }
    private struct ResolvedPayload: Decodable { let id: String; let chosen: String }
    private struct ModeSetPayload: Decodable {
        let agentMode: String
        enum CodingKeys: String, CodingKey { case agentMode = "agent_mode" }
    }
    private struct TurnEndedPayload: Decodable { let reason: String }
    private struct GapPayload: Decodable { let reason: GapReason }
}
```

Add the dependency in `apps/macos/Package.swift` — add to `Package(...)`:

```swift
    dependencies: [.package(path: "../shared/AgentKit")],
```

and to the `Far Cooler` executable target's `dependencies`:

```swift
            dependencies: ["CFarCoolerVT", .product(name: "AgentKit", package: "AgentKit")],
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/shared/AgentKit && swift test`
Expected: PASS, 3 tests

Then confirm the Mac app still builds: `cd apps/macos && swift build`

- [ ] **Step 5: Commit**

```bash
git add apps/shared/AgentKit apps/macos/Package.swift
git commit -m "feat(agentkit): one decoder both apps share, and an unknown event is a gap"
```

---

### Task 2: The transcript reducer

**Files:**
- Create: `apps/shared/AgentKit/Sources/AgentKit/Transcript.swift`
- Create: `apps/shared/AgentKit/Tests/AgentKitTests/TranscriptTests.swift`

**Interfaces:**
- Consumes: `AgentEvent`, `Sequenced` (Task 1).
- Produces: `AgentKit.Transcript` with `apply(_ events: [Sequenced])`, `rows: [TranscriptRow]`, `plan: [PlanEntry]`, `pendingPermission: PendingPermission?`, `agentMode: String?`, `availableModes: [String]`, `availableCommands: [String]`, `cursor: UInt64`.
- Produces: `AgentKit.TranscriptRow` (enum) and `AgentKit.PendingPermission`.

- [ ] **Step 1: Write the failing test**

Create `apps/shared/AgentKit/Tests/AgentKitTests/TranscriptTests.swift`:

```swift
import Testing
@testable import AgentKit

private func seq(_ n: UInt64, _ e: AgentEvent) -> Sequenced { Sequenced(seq: n, event: e) }

@Test func consecutiveChunksOfOneMessageBecomeOneRow() {
    // The agent streams a sentence as many chunks. One row per chunk would
    // render a column of one-word paragraphs.
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "Hello, ")),
        seq(1, .message(role: .agent, text: "world")),
    ])
    #expect(t.rows.count == 1)
    guard case let .message(role, text) = t.rows[0] else {
        Issue.record("expected one message row")
        return
    }
    #expect(role == .agent)
    #expect(text == "Hello, world")
}

@Test func aRoleChangeStartsANewRow() {
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "a")),
        seq(1, .message(role: .thought, text: "b")),
    ])
    #expect(t.rows.count == 2)
}

@Test func aToolUpdateMutatesItsCallRatherThanAppending() {
    // A tool that reports progress four times must stay one row, or the
    // transcript fills with duplicates of the same call.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "t1", title: "Edit main.rs", kind: "edit", status: .pending, locations: [])),
        seq(1, .toolUpdate(id: "t1", status: .completed, content: nil, diff: nil)),
    ])
    #expect(t.rows.count == 1)
    guard case let .tool(tool) = t.rows[0] else {
        Issue.record("expected a tool row")
        return
    }
    #expect(tool.status == .completed)
}

@Test func aDiffAttachesToTheToolItBelongsTo() {
    var t = Transcript()
    let diff = Diff(path: "main.rs", oldText: "old", newText: "new")
    t.apply([
        seq(0, .toolCall(id: "t1", title: "Edit", kind: "edit", status: .pending, locations: [])),
        seq(1, .toolUpdate(id: "t1", status: .completed, content: nil, diff: diff)),
    ])
    guard case let .tool(tool) = t.rows[0] else {
        Issue.record("expected a tool row")
        return
    }
    #expect(tool.diff == diff)
}

@Test func aGapIsItsOwnRowAndIsNeverMergedAway() {
    // If a gap could merge into a neighbouring message the user would never
    // learn that history is missing, which is the one thing this design
    // promises never to hide.
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "a")),
        seq(1, .gap(.ringTrimmed)),
        seq(2, .message(role: .agent, text: "b")),
    ])
    #expect(t.rows.count == 3)
    guard case .gap = t.rows[1] else {
        Issue.record("the gap must survive as its own row")
        return
    }
}

@Test func aPendingPermissionIsExposedAndClearedWhenAnswered() {
    var t = Transcript()
    let options = [PermissionOption(id: "allow", name: "Allow", kind: "allow_once")]
    t.apply([seq(0, .permission(id: "r1", toolCall: "t1", options: options))])
    #expect(t.pendingPermission?.id == "r1")

    t.apply([seq(1, .resolved(id: "r1", chosen: "allow"))])
    #expect(t.pendingPermission == nil)
}

@Test func thePlanIsReplacedWholesaleNotAppended() {
    // The daemon sends the whole plan every time, so appending would show
    // every historical version of the todo list stacked up.
    var t = Transcript()
    t.apply([seq(0, .plan(entries: [PlanEntry(content: "one", priority: "high", status: "pending")]))])
    t.apply([seq(1, .plan(entries: [PlanEntry(content: "two", priority: "high", status: "done")]))])
    #expect(t.plan.count == 1)
    #expect(t.plan[0].content == "two")
}

@Test func theCursorTracksTheHighestSeqSeen() {
    // Reconnect asks for everything after this. An off-by-one repeats a
    // message or skips one.
    var t = Transcript()
    t.apply([seq(0, .message(role: .agent, text: "a")), seq(7, .message(role: .user, text: "b"))])
    #expect(t.cursor == 8)
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/shared/AgentKit && swift test`
Expected: FAIL — `cannot find 'Transcript' in scope`

- [ ] **Step 3: Write the reducer**

Create `apps/shared/AgentKit/Sources/AgentKit/Transcript.swift`:

```swift
import Foundation

public struct ToolRow: Sendable, Equatable, Identifiable {
    public let id: String
    public var title: String
    public var kind: String
    public var status: ToolStatus
    public var locations: [String]
    public var content: String?
    public var diff: Diff?
}

public enum TranscriptRow: Sendable, Equatable, Identifiable {
    case message(role: Role, text: String)
    case tool(ToolRow)
    case gap(GapReason)

    public var id: String {
        switch self {
        case let .tool(t): "tool-\(t.id)"
        case let .message(role, text): "msg-\(role.rawValue)-\(text.hashValue)"
        case let .gap(reason): "gap-\(reason.rawValue)"
        }
    }
}

public struct PendingPermission: Sendable, Equatable {
    public let id: String
    public let toolCall: String
    public let options: [PermissionOption]
}

/// Events in, something renderable out.
///
/// Shared rather than written twice, because a phone and a Mac that reduced
/// the same events differently would disagree about one session — the exact
/// failure the daemon-side derivation model exists to prevent.
public struct Transcript: Sendable {
    public private(set) var rows: [TranscriptRow] = []
    public private(set) var plan: [PlanEntry] = []
    public private(set) var pendingPermission: PendingPermission?
    public private(set) var agentMode: String?
    public private(set) var availableModes: [String] = []
    public private(set) var availableCommands: [String] = []
    /// The seq to ask for on reconnect: one past the highest seen.
    public private(set) var cursor: UInt64 = 0

    public init() {}

    public mutating func apply(_ events: [Sequenced]) {
        for item in events {
            cursor = max(cursor, item.seq + 1)
            apply(item.event)
        }
    }

    private mutating func apply(_ event: AgentEvent) {
        switch event {
        case let .sessionStarted(_, mode, modes, commands):
            agentMode = mode
            availableModes = modes
            // Only replace when the session actually offered some. A later
            // event carrying an empty list would otherwise empty the picker.
            if !commands.isEmpty { availableCommands = commands }

        case let .message(role, text):
            // Chunks of one message coalesce. One row per chunk would render a
            // streamed sentence as a column of one-word paragraphs.
            if case let .message(lastRole, lastText) = rows.last, lastRole == role {
                rows[rows.count - 1] = .message(role: role, text: lastText + text)
            } else {
                rows.append(.message(role: role, text: text))
            }

        case let .toolCall(id, title, kind, status, locations):
            rows.append(.tool(ToolRow(
                id: id, title: title, kind: kind, status: status,
                locations: locations, content: nil, diff: nil)))

        case let .toolUpdate(id, status, content, diff):
            // Mutate the call in place. Appending would fill the transcript
            // with duplicates of one tool reporting progress.
            guard let index = rows.lastIndex(where: {
                if case let .tool(t) = $0 { return t.id == id }
                return false
            }) else {
                rows.append(.tool(ToolRow(
                    id: id, title: id, kind: "", status: status,
                    locations: [], content: content, diff: diff)))
                return
            }
            guard case var .tool(tool) = rows[index] else { return }
            tool.status = status
            if let content { tool.content = content }
            if let diff { tool.diff = diff }
            rows[index] = .tool(tool)

        case let .plan(entries):
            // Wholesale, because the daemon sends the whole plan each time.
            plan = entries

        case let .permission(id, toolCall, options):
            pendingPermission = PendingPermission(id: id, toolCall: toolCall, options: options)

        case let .resolved(id, _):
            if pendingPermission?.id == id { pendingPermission = nil }

        case let .modeSet(mode):
            agentMode = mode

        case .turnEnded:
            // Nothing to draw. The row's activity badge is the daemon's to
            // decide and arrives on the terminal, not here.
            break

        case let .gap(reason):
            // Never merged, never dropped. A gap that could be swallowed by a
            // neighbouring message would leave the user believing a transcript
            // is complete when it is not.
            rows.append(.gap(reason))
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/shared/AgentKit && swift test`
Expected: PASS, 11 tests

- [ ] **Step 5: Commit**

```bash
git add apps/shared/AgentKit
git commit -m "feat(agentkit): reduce events to rows, and never merge a gap away"
```

---

### Task 3: The composer's parse of `/` and `@`

**Files:**
- Create: `apps/shared/AgentKit/Sources/AgentKit/Composer.swift`
- Create: `apps/shared/AgentKit/Tests/AgentKitTests/ComposerTests.swift`

**Interfaces:**
- Produces: `AgentKit.ComposerToken` (enum: `.none`, `.slash(prefix:range:)`, `.mention(prefix:range:)`) and `AgentKit.activeToken(in:cursor:) -> ComposerToken`.

- [ ] **Step 1: Write the failing test**

```swift
import Testing
@testable import AgentKit

@Test func aSlashAtTheStartOpensTheCommandPicker() {
    let token = activeToken(in: "/mod", cursor: 4)
    guard case let .slash(prefix, _) = token else {
        Issue.record("expected a slash token, got \(token)")
        return
    }
    #expect(prefix == "mod")
}

@Test func aSlashMidSentenceIsJustASlash() {
    // Otherwise typing a file path or "and/or" pops a command menu over the
    // text the user is writing.
    let token = activeToken(in: "read src/main.rs", cursor: 16)
    guard case .none = token else {
        Issue.record("a slash inside a word must not open the picker")
        return
    }
}

@Test func anAtSignOpensTheFilePickerAnywhere() {
    // Unlike a slash command, a file mention is legitimate mid-sentence.
    let token = activeToken(in: "look at @src/ma", cursor: 15)
    guard case let .mention(prefix, _) = token else {
        Issue.record("expected a mention token, got \(token)")
        return
    }
    #expect(prefix == "src/ma")
}

@Test func anEmailAddressDoesNotOpenTheFilePicker() {
    // An @ preceded by a word character is part of that word.
    let token = activeToken(in: "mail me@example.com", cursor: 19)
    guard case .none = token else {
        Issue.record("an @ inside a word must not open the picker")
        return
    }
}

@Test func aSpaceClosesTheToken() {
    let token = activeToken(in: "@src/main.rs and then", cursor: 21)
    guard case .none = token else {
        Issue.record("the token ends at whitespace")
        return
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/shared/AgentKit && swift test`
Expected: FAIL — `cannot find 'activeToken' in scope`

- [ ] **Step 3: Write the parser**

```swift
import Foundation

public enum ComposerToken: Sendable, Equatable {
    case none
    case slash(prefix: String, range: Range<String.Index>)
    case mention(prefix: String, range: Range<String.Index>)
}

/// What the caret is currently inside, if anything.
///
/// Shared because getting it subtly different on two platforms means the
/// picker opens in one app and not the other for the same keystrokes.
public func activeToken(in text: String, cursor: Int) -> ComposerToken {
    guard cursor >= 0, cursor <= text.count else { return .none }
    let caret = text.index(text.startIndex, offsetBy: cursor)
    let head = text[text.startIndex..<caret]

    // Scan back to whitespace: a token cannot span a space.
    let tokenStart = head.lastIndex(where: { $0.isWhitespace }).map { text.index(after: $0) }
        ?? text.startIndex
    let token = text[tokenStart..<caret]
    guard let first = token.first else { return .none }

    if first == "/" {
        // Only at the very start of the message. A slash mid-sentence is a
        // path separator or an "and/or", and popping a command menu over that
        // interrupts ordinary writing.
        guard tokenStart == text.startIndex else { return .none }
        return .slash(prefix: String(token.dropFirst()), range: tokenStart..<caret)
    }

    if first == "@" {
        // Legitimate mid-sentence, unlike a slash — but not when it is part of
        // a word, which is what an email address is.
        return .mention(prefix: String(token.dropFirst()), range: tokenStart..<caret)
    }

    return .none
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/shared/AgentKit && swift test`
Expected: PASS, 16 tests

- [ ] **Step 5: Commit**

```bash
git add apps/shared/AgentKit
git commit -m "feat(agentkit): one parse of slash and at, so both apps pop the same picker"
```

---

### Task 4: Carry the new terminal fields to the clients

**Files:**
- Modify: `crates/client/src/session.rs`
- Modify: `apps/macos/Sources/FarCooler/Model.swift`
- Modify: `apps/ios/FarCooler/Model.swift`

**Interfaces:**
- Produces: `paneMode`, `agentSessionId`, `agentMode`, `availableAgentModes` on the Swift `Terminal` in both apps.

`crates/client/src/session.rs` builds the JSON both apps decode (see the `"preset"`/`"activity"` construction around line 183). Add the four fields there, mapping `PaneMode` to the strings `"terminal"` and `"agent"`.

- [ ] **Step 1: Write the failing test**

Add to `crates/client/src/session.rs`'s test module:

```rust
    #[test]
    fn a_terminal_reports_its_pane_mode_as_a_word_a_client_can_switch_on() {
        // Numbers would make every client carry a copy of the enum and drift
        // from it. The label is the daemon's answer, not a code to look up.
        assert_eq!(pane_mode_label(farcooler_protocol::v1::PaneMode::Terminal as i32), "terminal");
        assert_eq!(pane_mode_label(farcooler_protocol::v1::PaneMode::Agent as i32), "agent");
        // An unknown value is the mode that always works, not a guess.
        assert_eq!(pane_mode_label(99), "terminal");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-client pane_mode`
Expected: FAIL — `cannot find function pane_mode_label`

- [ ] **Step 3: Implement**

In `crates/client/src/session.rs`, beside `activity_label`:

```rust
/// The pane mode, as a word rather than a number.
///
/// Same reason `activity_label` exists: a client that switched on an integer
/// would hold a second copy of the enum and drift from it silently.
fn pane_mode_label(mode: i32) -> &'static str {
    match farcooler_protocol::v1::PaneMode::try_from(mode) {
        Ok(farcooler_protocol::v1::PaneMode::Agent) => "agent",
        // Unspecified from an older daemon is terminal: the mode that needs no
        // adapter and always works.
        _ => "terminal",
    }
}
```

Add to the JSON object built around line 183:

```rust
                            "paneMode": pane_mode_label(t.pane_mode),
                            "agentSessionId": t.agent_session_id.clone(),
                            "agentMode": t.agent_mode.clone(),
                            "availableAgentModes": t.available_agent_modes.clone(),
```

Add to `struct Terminal` in BOTH `apps/macos/Sources/FarCooler/Model.swift` and `apps/ios/FarCooler/Model.swift`:

```swift
    /// What this terminal's pane is hosting. Absent on older daemons, which is
    /// why it is optional rather than defaulted to something that would look
    /// like a real answer.
    var paneMode: String?
    var agentSessionId: String?
    var agentMode: String?
    var availableAgentModes: [String]?

    /// Whether to draw a chat or a VT grid.
    var isAgentPane: Bool { paneMode == "agent" }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-client`
Expected: PASS

Then: `cd apps/macos && swift build`

- [ ] **Step 5: Commit**

```bash
git add crates/client apps/macos/Sources/FarCooler/Model.swift apps/ios/FarCooler/Model.swift
git commit -m "feat(client): a terminal says which surface to draw"
```

---

### Task 5: The agent stream on the client side

**Files:**
- Modify: `crates/client/src/session.rs`
- Modify: `crates/client/src/ffi.rs`
- Create: `apps/macos/Sources/FarCooler/AgentStream.swift`

**Interfaces:**
- Produces: a client call that subscribes from a cursor and returns `[(seq, payload_json)]`, and `AgentStream`, an `ObservableObject` holding a `Transcript` for one terminal.

Read `crates/client/src/ffi.rs` and `apps/macos/Sources/FarCooler/TerminalStream.swift` first — the terminal stream is the pattern this follows, and the FFI boundary has an established shape that must be matched rather than reinvented.

- [ ] **Step 1: Write the failing test**

Add to `crates/client/tests/against_a_real_daemon.rs`:

```rust
#[tokio::test]
async fn subscribing_to_a_terminal_with_no_agent_session_is_empty_not_an_error() {
    // A client attaches to a PANE, not to a session. A terminal that has never
    // been in agent mode must answer "nothing yet" rather than fail, or the
    // UI cannot open a chat view before the first turn.
    let harness = Harness::start().await;
    let terminal = harness.create_terminal("shell").await;
    let batch = harness.agent_subscribe(&terminal.id, 0).await.expect("subscribe succeeds");
    assert!(batch.events.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-client --test against_a_real_daemon subscribing`
Expected: FAIL — no `agent_subscribe` method

- [ ] **Step 3: Implement**

Add `agent_subscribe`, `agent_prompt`, `agent_answer`, `agent_set_mode`, `agent_cancel`, `set_pane_mode` and `worktree_file_search` to the client session, each a thin wrapper over the request payloads added in the core plan's Task 16, following the existing method style exactly. Expose them across the FFI in the same shape `ffi.rs` already uses for terminal calls.

Create `apps/macos/Sources/FarCooler/AgentStream.swift`:

```swift
import AgentKit
import Foundation

/// One terminal's agent session, as the view needs it.
///
/// Holds a `Transcript` and nothing else derived: activity, pane mode and the
/// agent's modes all arrive on the `Terminal` from the daemon, because two
/// clients deciding those for themselves is the disagreement this whole design
/// exists to prevent.
@MainActor
final class AgentStream: ObservableObject {
    @Published private(set) var transcript = Transcript()
    @Published private(set) var connectionError: String?

    private let terminal: String
    private var pollTask: Task<Void, Never>?

    init(terminal: String) {
        self.terminal = terminal
    }

    func start() {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.pump()
                try? await Task.sleep(for: .milliseconds(200))
            }
        }
    }

    func stop() {
        pollTask?.cancel()
        pollTask = nil
    }

    /// Ask for everything after what we already hold.
    ///
    /// The cursor comes from the transcript rather than from a counter kept
    /// here, so a reconnect cannot skip or repeat events after a gap.
    private func pump() async {
        do {
            let batch = try await DaemonClient.shared.agentSubscribe(
                terminal: terminal, fromSeq: transcript.cursor)
            guard !batch.isEmpty else { return }
            let decoded = batch.compactMap { frame -> Sequenced? in
                guard let event = try? AgentEvent.decode(from: frame.payloadJSON) else { return nil }
                return Sequenced(seq: frame.seq, event: event)
            }
            transcript.apply(decoded)
            connectionError = nil
        } catch {
            connectionError = String(describing: error)
        }
    }

    func send(_ text: String) async { try? await DaemonClient.shared.agentPrompt(terminal: terminal, text: text) }
    func answer(_ requestID: String, _ optionID: String) async {
        try? await DaemonClient.shared.agentAnswer(terminal: terminal, requestID: requestID, optionID: optionID)
    }
    func setMode(_ mode: String) async { try? await DaemonClient.shared.agentSetMode(terminal: terminal, mode: mode) }
    func cancel() async { try? await DaemonClient.shared.agentCancel(terminal: terminal) }
}
```

Match `DaemonClient`'s real method names and error handling to what exists in `apps/macos/Sources/FarCooler/DaemonClient.swift`; the names above are the shape, not a promise about that file's API.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-client` then `cd apps/macos && swift build`
Expected: PASS and a clean build

- [ ] **Step 5: Commit**

```bash
git add crates/client apps/macos/Sources/FarCooler/AgentStream.swift
git commit -m "feat(client): subscribe to an agent session by cursor"
```

---

### Task 6: `AgentSurface` — the pane

**Files:**
- Create: `apps/macos/Sources/FarCooler/AgentSurface.swift`
- Create: `apps/macos/Sources/FarCooler/AgentRows.swift`
- Create: `apps/macos/Sources/FarCooler/DiffView.swift`
- Modify: `apps/macos/Sources/FarCooler/TileView.swift`
- Modify: `apps/macos/Sources/FarCooler/TerminalPane.swift`

**Interfaces:**
- Consumes: `AgentStream` (Task 5), `Terminal.isAgentPane` (Task 4), `AgentKit.TranscriptRow`.

Read `TerminalSurface.swift`, `TerminalPane.swift` and `TileView.swift` first. `TileView` computes no arrangement — tmux reports pane rectangles and the view scales them. An agent pane occupies a real tmux rectangle, so **no layout code changes**: the only edit is choosing which surface to draw inside the existing frame.

- [ ] **Step 1: Write the failing test**

macOS views are not unit-testable here without adding a UI test harness, which is out of scope. Instead assert the routing decision, which is the part that can silently be wrong. Add to a new `apps/macos/Sources/FarCooler/AgentSurface.swift` test-free helper and cover it in `AgentKit`:

Add to `apps/shared/AgentKit/Tests/AgentKitTests/TranscriptTests.swift`:

```swift
@Test func aTranscriptWithOnlyAGapStillHasSomethingToDraw() {
    // The empty-state and the gap-state are different. Rendering "no messages
    // yet" over a gap would tell the user nothing happened when in fact
    // something happened and was lost.
    var t = Transcript()
    t.apply([Sequenced(seq: 0, event: .gap(.loadUnsupported))])
    #expect(t.rows.count == 1)
    #expect(t.rows.isEmpty == false)
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/shared/AgentKit && swift test`
Expected: PASS already if Task 2 is correct — this is a guard, not a new behavior. If it fails, Task 2's gap handling is wrong; fix that first.

- [ ] **Step 3: Build the views**

`AgentRows.swift` renders one `TranscriptRow`:
- `.message(.user, …)` — right-aligned, secondary background
- `.message(.agent, …)` — plain body text
- `.message(.thought, …)` — a `DisclosureGroup`, collapsed by default
- `.tool(row)` — one line: a status dot, the title, and an expander revealing `content` and, when present, `DiffView`
- `.gap(reason)` — a full-width rule with a short sentence naming the reason. Never subtle: this is the transcript admitting it is incomplete.

`DiffView.swift` renders a `Diff` as a unified diff computed from `oldText`/`newText`, with add/remove backgrounds, line numbers, and collapse-by-default beyond 20 lines. No syntax highlighting.

`AgentSurface.swift` composes: an optional plan panel (when `transcript.plan` is non-empty), the scrolling transcript pinned to the bottom, an inline approval card when `transcript.pendingPermission != nil`, and the composer row. The approval card's buttons come from the permission's own options, bound to `1`/`2`/`3` and return.

In `TileView.swift`, where a pane's content is chosen, switch on the terminal:

```swift
if terminal.isAgentPane {
    AgentSurface(terminal: terminal, workspace: workspace)
} else {
    TerminalSurface(/* unchanged */)
}
```

Do the same in `TerminalPane.swift` for the single-pane case. Both keep `.paneCard()` and `.paneCanvas()` exactly as they are.

An agent pane must still report cell dimensions through the existing `onViewport` path, or tmux lays the window out against stale numbers and every terminal tile beside it is sized wrongly. Compute them from the surface's pixel size using the same font metrics `TerminalSurface` uses.

- [ ] **Step 4: Verify**

Run: `cd apps/macos && swift build`
Then run the app, create a workspace, launch a claude terminal, and switch it to agent mode from the command palette. Confirm: the chat renders in the tile, tiling a second terminal beside it does not disturb it, `⌃L` moves focus in and out, and an approval prompt appears as a card with working buttons.

- [ ] **Step 5: Commit**

```bash
git add apps/macos/Sources/Far Cooler
git commit -m "feat(macos): a chat is a pane, drawn where a terminal would be"
```

---

### Task 7: Composer, slash commands, mentions, images, mode switcher

**Files:**
- Create: `apps/macos/Sources/FarCooler/AgentComposer.swift`
- Modify: `apps/macos/Sources/FarCooler/AgentSurface.swift`
- Modify: `apps/macos/Sources/FarCooler/Shortcuts.swift`
- Modify: `apps/macos/Sources/FarCooler/CommandPalette.swift`

**Interfaces:**
- Consumes: `AgentKit.activeToken` (Task 3), `transcript.availableCommands`, `transcript.availableModes`, the worktree file-search call (Task 5).

- [ ] **Step 1 through 5**

Build `AgentComposer`: a text field that consults `activeToken(in:cursor:)` on every change and shows the existing `PaletteIndex`-backed picker for `.slash` (fed by `transcript.availableCommands`) and `.mention` (fed by the worktree file-search RPC, debounced). Accepting a completion replaces the token's range.

The composer row also carries the agent mode control — a menu listing `transcript.availableModes` with the current one checked, calling `agentSetMode` — and an attach button plus drag-and-drop and paste for images, which become image blocks on the prompt.

The terminal↔chat toggle is a command, not chrome: add it to `Shortcuts.swift` as a `⌃B` binding and to `CommandPalette.swift`, calling `setPaneMode`. When the daemon answers `ConfirmationRequired` because a turn is in flight, show a confirmation naming what will be cancelled, then retry with `force: true`.

Verify with `swift build` and by exercising each affordance against a live agent. Commit as `feat(macos): compose with commands, mentions, images and a mode`.

---

### Task 8: iOS

**Files:**
- Create: `apps/ios/FarCooler/AgentView.swift`
- Create: `apps/ios/FarCooler/AgentStream.swift`
- Modify: `apps/ios/FarCooler/TerminalView.swift`
- Modify: `apps/ios/generate-project.py`

**Interfaces:**
- Consumes: everything above.

`apps/ios` uses a generated Xcode project whose `SOURCES` list is explicit. Add the new files there, and add the shared `AgentKit` sources as their own group with path `../shared/AgentKit/Sources/AgentKit` — the generator currently assumes basenames under `Far Cooler/`, so this needs a second group in the same style as the existing `fontsGroup`. Regenerate with `./apps/ios/generate-project.py`.

The view is the same surface without tiling: a full-screen agent view swapped with the terminal view behind the existing `TerminalTabStrip`, chosen by `terminal.isAgentPane`. The composer sits over the keyboard using the existing key-row work; images come from the photo picker and paste. The approval card is the same shape as the Mac's, sized for a thumb.

Verify by building and running on a device, driving a real agent from the phone through an approval and a mode switch. Commit as `feat(ios): the agent view, where the terminal was`.

---

## What this plan deliberately does not do

- No syntax highlighting in diffs, no per-hunk accept/reject, no checkpoint/rewind — cut in the spec.
- No Android. The React Native path in the design doc is a later decision.
- No new pane chrome. The composer row and the existing command surfaces carry everything.
