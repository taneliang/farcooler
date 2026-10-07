#if DEBUG
import SwiftUI
import UIKit

/// The conversation view of a terminal-mode claude pane (ov-373), on the
/// app's own route with nothing dialed: `ShellScreen` over a canned
/// `Connection`, the real `TerminalView`, `NativeSwitch` and
/// `NativeAgentView`, reading rows from a canned runner through the client
/// core's own call path (`ClientCore.standIn`). `-native-agent-harness`.
///
/// What the canned runner does, and the flags that change it:
/// - `agent.rows` answers a page of every row kind, in the client core's
///   JSON. A follow waits a moment and answers what's new: the reply
///   (`prose:a1:0`) changed in place once "change the reply" is sent, and a
///   turn for each other message sent.
/// - `terminal.compose` takes the message and the next follow shows it as a
///   turn; `-native-busy` answers that claude's queue took it,
///   `-native-dialog` refuses it for a dialog, `-native-draft` for a draft in
///   the terminal's box.
/// - `-native-flag-off`: a runner whose projector is off, so no `agent_rows`.
/// - `-native-reconnect`: once the box holds a draft, the link comes up
///   again, so the build is unread for two seconds.
/// - `-native-off-on`: on the fourth follow, the projector is turned off
///   (rows refused, a hello without `agent_rows`), and back on once the pane
///   has shown its terminal for it.
/// - `-native-stale`: from the fourth follow on, every rows call is lost.
/// - Compose's text picks a failure: "time out", "garble" (an unreadable
///   answer) and "read only" (a grant that may not type).
/// - `-native-terminal`: the pane last switched to its terminal (R-27).
///
/// `native-harness` reads back what was sent, and how many follows were
/// asked for, for the tests.
struct NativeAgentHarness: View {
    static var isRequested: Bool { CommandLine.arguments.contains("-native-agent-harness") }

    @StateObject private var world = NativeHarnessWorld()
    @StateObject private var hosts = RunnerStore()
    private static let harnessRunner = Runner(label: "Conversation harness", address: "harness.invalid", user: "harness")
    static let pane = "11111111-2222-4333-8444-555555555555"

    private var connection: Connection { world.connection }
    private var fleetStore: FleetStore { world.fleetStore }
    private var runner: NativeHarnessRunner { world.runner }

    /// Each launch starts on the pane's default view unless a flag says
    /// otherwise, and with no cached rows from the last. Once per launch,
    /// never in `init`: this view is built again whenever the app's body is,
    /// and a defaults write there changes every `@AppStorage` reader, which
    /// rebuilds the app's body, which builds this again: a loop that held the
    /// main thread at about 70% and timed out UI queries on CI.
    private static let preparedOnce: Void = {
        AgentConversation.remember(
            conversation: !CommandLine.arguments.contains("-native-terminal"), for: pane)
        if let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first {
            try? FileManager.default.removeItem(at: caches.appendingPathComponent("agent-rows/phone-\(pane).json"))
        }
    }()

    /// What the harness stands on, made once for the view's life. A
    /// `Connection` built in `init` started a client core, a whole runtime,
    /// every time the app's body ran.
    @MainActor
    final class NativeHarnessWorld: ObservableObject {
        let connection: Connection
        let fleetStore: FleetStore
        let runner = NativeHarnessRunner()

        init() {
            _ = NativeAgentHarness.preparedOnce
            connection = Connection()
            fleetStore = FleetStore.standIn(on: connection, host: NativeAgentHarness.harnessRunner)
        }
    }

    var body: some View {
        ShellScreen(
            fleet: fleetStore, hosts: hosts, pendingTerminal: .constant(nil),
            scope: ShellScope(runner: Self.harnessRunner.id, worktree: Self.worktree.id, landing: .terminal(Self.pane)))
            // In a view of its own, so a follow's report redraws one probe
            // and not the whole shell.
            .overlay(alignment: .topLeading) { NativeHarnessProbe(runner: runner) }

            .task { await stand() }
    }

    private func stand() async {
        let runner = runner
        await connection.core.standIn { method, args in try await runner.answer(method, args) }
        connection.standIn(
            on: Fleet(runtimeHealthy: true, livePanes: 2, worktrees: [Self.worktree]),
            repositories: [],
            build: Self.build(rows: !CommandLine.arguments.contains("-native-flag-off")))
        fleetStore.republish()
        let connection = connection
        runner.links = { rows in
            // A link coming up again: no build for a round trip, then the
            // new hello's.
            connection.standInLinkCameUp()
            try? await Task.sleep(for: .seconds(2))
            connection.standInBuildLanded(Self.build(rows: rows))
        }
    }

    /// The runner's build: with its projector on, `agent_rows` and compose.
    static func build(rows: Bool) -> DaemonBuild {
        DaemonBuild(
            version: "harness", matches: true, platform: "harness",
            capabilities: Set(
                ["workspaces", "terminals", "agent", "projector_setting"] + (rows ? ["agent_rows", "agent_compose"] : [])))
    }

    private static var worktree: Worktree {
        Worktree(
            id: "native-ws", short: "native", task: "Conversation harness", branch: "fixture · no runner",
            state: "ready",
            terminals: [
                Terminal(
                    id: pane, short: "claude", title: "claude", preset: "claude", state: "running",
                    activity: "working", epoch: 1, paneMode: "terminal", chatCapable: false),
                Terminal(
                    id: "native-shell", short: "shell", title: "shell", preset: "shell", state: "running", epoch: 1,
                    paneMode: "terminal"),
            ])
    }
}

/// What the canned runner has been asked, for the tests.
private struct NativeHarnessProbe: View {
    @ObservedObject var runner: NativeHarnessRunner

    var body: some View {
        Color.clear
            .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a 1 pt element the UI tests read, nothing drawn
            .accessibilityElement()
            .accessibilityIdentifier("native-harness")
            .accessibilityValue(runner.said)
    }
}

/// The canned runner behind `NativeAgentHarness`.
@MainActor
final class NativeHarnessRunner: ObservableObject {
    /// What the tests read: `follows=N background=N changed=B linked=N sent=a|b`.
    @Published private(set) var said = "follows=0 background=0 changed=false linked=0 sent="
    /// Links that came up again and whose build has landed.
    private var linked = 0
    private var follows = 0
    /// Follows asked for while the app wasn't in front.
    private var background = 0
    private var sent: [String] = []
    /// Messages sent and not yet shown by a follow.
    private var unshown: [String] = []
    private var rev: UInt64 = 10
    /// The link coming up again, with or without rows on the new hello.
    var links: ((Bool) async -> Void)?
    /// The projector is off: rows are refused.
    private var off = false
    /// The link is down for rows.
    private var lost = false
    /// Whether the reply's change is due (a test sent "change the reply",
    /// once it had seen the reply as it was), and whether it's been sent.
    private var due = false
    private var updated = false

    private static let epoch: UInt64 = 7

    nonisolated func answer(_ method: String, _ args: [String: Any]) async throws -> Data {
        switch method {
        case "agent.rows":
            try await MainActor.run { try refuseIfOff() }
            return try await MainActor.run { try json(page()) }
        case "agent.rows_follow":
            return try await follow()
        case "terminal.compose":
            let text = args["text"] as? String ?? ""
            return try await MainActor.run { try compose(text) }
        case "terminal.screen":
            let text = "claude is running in this terminal\r\n> "
            return try json([
                "contents": Data(text.utf8).base64EncodedString(), "columns": 40, "rows": 12, "cursorColumn": 2,
                "cursorRow": 1, "revision": 1, "unchanged": false,
            ])
        default:
            throw ClientCore.CoreError.rejected("not in the harness", word: "unimplemented")
        }
    }

    private func report() {
        said = "follows=\(follows) background=\(background) changed=\(updated) linked=\(linked) sent=\(sent.joined(separator: "|"))"
    }

    private func refuseIfOff() throws {
        if off { throw ClientCore.CoreError.rejected("Rows aren't served.", word: "capability-unsupported") }
        if lost { throw ClientCore.CoreError.disconnected("The link dropped.") }
    }

    /// A link coming up again, counted once its build lands.
    private func link(rows: Bool) async {
        await links?(rows)
        linked += 1
        report()
    }

    /// Wait for the app to reach `state`, polling, for up to a minute: a
    /// barrier, so nothing here races the view it's testing.
    private func until(_ state: @MainActor () -> Bool) async {
        for _ in 0..<600 where !state() {
            try? await Task.sleep(for: .milliseconds(100))
        }
    }

    /// What the fourth follow sets off, under its flag.
    private func onFourth() {
        let args = CommandLine.arguments
        let pane = NativeAgentHarness.pane
        if args.contains("-native-reconnect") {
            // Once the person is mid-sentence in the box.
            Task {
                await until { !(NativePanes.shared.existing(pane)?.draft.isEmpty ?? true) }
                await link(rows: true)
            }
        } else if args.contains("-native-off-on") {
            off = true
            Task {
                await link(rows: false)
                // On again only once the pane has shown its terminal for
                // being off.
                await until { (NativeProbe.dropped[pane] ?? 0) >= 1 }
                off = false
                await link(rows: true)
            }
        } else if args.contains("-native-stale") {
            lost = true
        }
    }

    private func compose(_ text: String) throws -> Data {
        let args = CommandLine.arguments
        switch text {
        case "time out": throw ClientCore.CoreError.timedOut("No answer in time.")
        case "garble": throw ClientCore.CoreError.malformed
        case "read only": throw ClientCore.CoreError.rejected("Not with this grant.", word: "scope-denied")
        case "change the reply":
            due = true
            return try json(["queued": false])
        default: break
        }
        if args.contains("-native-dialog") {
            throw ClientCore.CoreError.rejected("A dialog is open.", word: "resource-conflict", what: "dialog")
        }
        if args.contains("-native-draft") {
            throw ClientCore.CoreError.rejected("The box holds a draft.", word: "resource-conflict", what: "draft")
        }
        sent.append(text)
        report()
        let busy = args.contains("-native-busy")
        if !busy { unshown.append(text) }
        return try json(["queued": busy])
    }

    private func follow() async throws -> Data {
        try await MainActor.run {
            follows += 1
            if UIApplication.shared.applicationState != .active { background += 1 }
            report()
            if follows == 4 { onFourth() }
            try refuseIfOff()
        }
        try await Task.sleep(for: .milliseconds(700))
        return try await MainActor.run {
            var changes: [[String: Any]] = []
            if due, !updated {
                updated = true
                rev += 1
                changes.append([
                    "kind": "update", "id": "prose:a1:0", "rev": rev,
                    "row": Self.row("prose:a1:0", ord: 2, rev: rev, kind: ["Prose": ["text": Self.updatedReply, "conclusion": true]]),
                ])
            }
            for text in unshown {
                rev += 1
                let id = "turn:sent\(rev)"
                changes.append([
                    "kind": "insert", "id": id, "rev": rev,
                    "row": Self.row(id, ord: 100 + rev, rev: rev, kind: ["Turn": Self.turn(text, origin: "Typed", open: true)]),
                ])
            }
            unshown = []
            report()
            return try json(["epoch": Self.epoch, "rev": rev, "reset": false, "changes": changes])
        }
    }

    static let firstReply = "Reading the parser now."
    static let updatedReply = "Read the parser. It’s tidy now: three functions, no globals."

    private func page() -> [String: Any] {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let rows: [[String: Any]] = [
            Self.row("turn:p1", ord: 1, rev: 1, kind: ["Turn": Self.turn("Tidy the parser, please.", origin: "Typed")]),
            Self.row("prose:a1:0", ord: 2, rev: 2, kind: ["Prose": ["text": Self.firstReply, "conclusion": false]]),
            Self.row("tool:t1", ord: 3, rev: 3, kind: ["Tool": [
                "name": "Edit", "summary": "src/parser.rs", "status": "Done", "started_ms": now - 9000, "ended_ms": now - 8000,
                "file_path": "src/parser.rs",
                "diff": [["old_start": 1, "old_lines": 1, "new_start": 1, "new_lines": 1, "lines": ["-let x = 1;", "+let x = 2;"]]],
            ]]),
            Self.row("subagent:s1", ord: 4, rev: 4, kind: ["Subagent": [
                "tool_use_id": "s1", "agent_type": "general-purpose", "description": "Count the lines", "background": true,
                "status": "Completed", "started_ms": now - 60000, "ended_ms": now - 30000, "tool_count": 3,
                "current_action": "", "last_ms": now - 30000,
            ]]),
            Self.row("turn:n1", ord: 5, rev: 5, kind: ["Turn": Self.turn("Agent \"Count the lines\" finished", origin: "Notification")]),
            Self.row("notice:c1", ord: 6, rev: 6, kind: ["Notice": ["kind": "Compacted", "text": "Conversation compacted"]]),
            Self.row("ask:q1", ord: 7, rev: 7, kind: ["Ask": ["kind": "Question", "text": "Keep the old names?", "answered": true]]),
            Self.row("gap:g1", ord: 8, rev: 8, kind: ["Gap": ["reason": "Unparsed", "count": 2]]),
            Self.row("turn:p2", ord: 9, rev: 9, kind: ["Turn": Self.turn("And the lexer.", origin: "Typed", open: true)]),
            // No start time, so no timer ticks: a tree that changes every
            // second makes every XCUITest query slow on a loaded runner.
            Self.row("thinking:k1", ord: 10, rev: 10, kind: ["Thinking": [String: Any]()]),
        ]
        return ["epoch": Self.epoch, "rev": rev, "moreBefore": false, "rows": rows]
    }

    private static func turn(_ prompt: String, origin: String, open: Bool = false) -> [String: Any] {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        var turn: [String: Any] = ["prompt": prompt, "origin": origin, "background_running": 0]
        if open {
            // No start time: see the thinking row.
            turn["activity"] = "Busy"
        } else {
            turn["started_ms"] = now - 50000
            turn["ended_ms"] = now - 8000
            turn["duration_ms"] = 42000
            turn["outcome"] = "Finished"
        }
        return turn
    }

    private static func row(_ id: String, ord: UInt64, rev: UInt64, kind: [String: Any]) -> [String: Any] {
        ["id": id, "ord": ord, "rev": rev, "provisional": false, "kind": kind]
    }
}

private func json(_ object: Any) throws -> Data {
    try JSONSerialization.data(withJSONObject: object)
}
#endif
