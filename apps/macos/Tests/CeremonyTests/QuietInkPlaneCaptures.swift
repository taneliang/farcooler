import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the fifth batch of ov-687, for judging that
/// the hierarchy still reads. The views whose quiet text sits on the frosted
/// plane (the navigator's task row, the board's blank state, the tab strip, the
/// jump bar, the empty main area) are drawn over `WindowPlane`, and the ones on
/// a card over the paper, each in light and dark. For the before, run it on the
/// commit before the change.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct QuietInkPlaneCaptures {
    enum Ground { case plane, paper }

    static func shoot<V: View>(_ name: String, width: CGFloat, height: CGFloat, on ground: Ground, _ view: @autoclosure () -> V) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (variant, appearance) in QuietInkSurfaceCaptures.appearances {
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            let shown = view().frame(width: width, height: height, alignment: .topLeading)
            window.contentView = NSHostingView(
                rootView: AnyView(
                    ground == .plane
                        ? AnyView(shown.background { WindowPlane() })
                        : AnyView(shown.background(WorkspaceStyle.paper))))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            try await Capture.keep(
                to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-\(variant).png"),
                expecting: name, until: { true }
            ) { window.contentView?.lookBitmap(scale: 2)?.representation(using: .png, properties: [:]) }
            window.close()
        }
    }

    @Test func navigatorRow() async throws {
        let clock = Date()
        let since = clock.addingTimeInterval(-TaskRow.staleAfter + 60)
        let row = TaskRow(
            id: "t1", key: "ov-687", title: "Mac: secondary text reads in light appearance, the last of the sidebar and Settings",
            status: .inProgress, statusSince: since, createdAt: since, updatedAt: since)
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let store = TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
        try await Self.shoot(
            "quiet5-navigator-row", width: 280, height: 70, on: .plane,
            TaskListRow(row: row, prominent: false, store: store, live: [], presence: .unsaid, onGoTo: { _ in })
                .padding(.horizontal, 8))
    }

    @Test func boardBlank() async throws {
        try await Self.shoot("quiet5-board-blank", width: 280, height: 220, on: .plane, BoardBlankState())
    }

    @Test func nothingOpen() async throws {
        try await Self.shoot("quiet5-nothing-open", width: 520, height: 160, on: .plane, WorkspaceMain.nothingOpen)
    }

    @Test func groupBar() async throws {
        let panes = (0..<3).map { i in
            PaneRect(id: "p\(i)", short: "p\(i)", title: nil, left: i * 41, top: 0, columns: 39, rows: 24, focused: i == 0, zoomed: false)
        }
        let groups = ["Review", "Dev server", "Scratch"].enumerated().map { i, name in
            PaneGroup(id: "@\(i)", name: name, active: i == 0, columns: 120, rows: 24, layout: "a", panes: i == 1 ? panes : [panes[0]])
        }
        try await Self.shoot(
            "quiet5-group-bar", width: 520, height: 40, on: .plane,
            GroupBar(groups: groups, showing: "@0", onSelect: { _ in }).padding(.horizontal, 10))
    }

    @Test func breadcrumb() async throws {
        let go = WorkspaceNavigation.Crumb(title: "Main", target: .workspace(host: "", workspace: "ws", focus: nil))
        let task = WorkspaceNavigation.Crumb(title: "Mac: secondary text reads in light appearance", target: go.target, glyph: "checklist")
        try await Self.shoot(
            "quiet5-breadcrumb", width: 620, height: 40, on: .plane,
            DrillBreadcrumb(
                crumbs: [go, task], worktrees: WorktreeCrumb(title: "mac-687d", isHere: true, tasks: [], loose: []),
                onGo: { _ in }, onClose: {}))
    }

    @Test func terminalsStrip() async throws {
        let terminals = [
            Terminal(id: "a", short: "a", title: "npm run dev", preset: "shell", state: "RUNNING", epoch: 0),
            Terminal(id: "b", short: "b", title: "cargo watch -x test", preset: "shell", state: "RUNNING", epoch: 0),
        ]
        try await Self.shoot(
            "quiet5-terminals-strip", width: 520, height: 80, on: .paper, TaskTerminalsStrip(terminals: terminals, onThisMac: true))
    }

    @Test func lostTerminal() async throws {
        let terminal = Terminal(id: "a", short: "a", title: "claude --resume 1234", preset: "claude", state: "LOST", epoch: 0)
        try await Self.shoot(
            "quiet5-lost-terminal", width: 700, height: 420, on: .paper,
            LostTerminalPage(terminal: terminal, hasKeyboard: false, onAction: { _ in }))
    }

    @Test func needsYou() async throws {
        let item = NeedsYouItem(
            id: "ask:x", kind: .ask, rank: 1, since: Date().addingTimeInterval(-5_400), workspaceID: "ws", workspaceName: "Billing",
            repositoryID: "r", task: nil, terminal: nil, question: "Allow the agent to run the migration against staging?",
            detail: "psql -h staging.internal -f migrations/0042_invoice_totals.sql", askID: nil, actions: [])
        try await Self.shoot(
            "quiet5-needs-you", width: 760, height: 360, on: .paper,
            NeedsYouView(
                items: [item], olderRunners: ["studio"], canAct: { _ in true }, onOpen: { _ in },
                onAnswerAsk: { _, _ in nil }, onDecide: { _, _ in true }))
    }
}
