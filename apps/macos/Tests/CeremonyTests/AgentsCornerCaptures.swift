import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the agents in the orchestrator's corner
/// (ov-829), in the production `WorkspaceView` in a real titled window with a
/// toolbar, taken with `screencapture -l`. The orchestrator's pane is the
/// production `NativeSwitch` showing its conversation, with a queued message
/// and its Send Now. Beside the canvas, a shell opened under its breadcrumb,
/// at the chat's least and most width; folded, the plan's Now/Next over the
/// chat. Light and dark, increased contrast too, with 1 agent and with 12,
/// and the Agents panel open. The panel is drawn in place under its control:
/// a real popover is a window of its own, which `screencapture -l` leaves out.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct AgentsCornerCaptures {
    typealias T = NativeAgentTests
    typealias W = WorkspaceAgentsTests

    /// A runner that says it presses keys, so Send Now is offered.
    actor Keys: InterruptSink {
        func interrupt(terminal: String) async throws {}
        func sendNow(terminal: String) async throws {}
    }

    static let lanes: [(id: String, title: String, line: String, state: String, failed: Bool)] = [
        ("l1", "Build ov-829 agents bar", "Edit AgentsCorner.swift", "working", false),
        ("l2", "Review ov-807 plan band options", "Allow Bash swift test --filter Band?", "blocked", false),
        ("l3", "Fix ov-640 relay retries", "cargo test failed: 3 errors", "done", true),
        ("l4", "Build ov-753 member roles", "Read RunnerPeople.swift", "working", false),
        ("l5", "Build ov-812 Android queue words", "Bash ./gradlew :app:testDebugUnitTest", "working", false),
        ("l6", "Review ov-801 watch complications", "Read ComplicationController.swift", "working", false),
        ("l7", "Build ov-818 teams page copy", "Edit site/src/pages/teams.astro", "working", false),
        ("l8", "Build ov-822 iPad sidebar width", "Grep sidebarWidth", "working", false),
        ("l9", "Fix ov-826 diff cache eviction", "Bash swift test --filter DiffViewCache", "working", false),
        ("l10", "Build ov-830 relay migration", "Allow Edit migrations/0042_roles.sql?", "blocked", false),
        ("l11", "Review ov-815 composer paste", "Read ComposerTextView.swift", "working", false),
        ("l12", "Build ov-831 plan peek motion", "Edit PlanPeek.swift", "working", false),
    ]

    static func fleet(agents: Int) throws -> Fleet {
        let turn = Date().timeIntervalSince1970 * 1000 - 185_000
        let lanes = Self.lanes.prefix(agents).map { lane in
            W.worktree(lane.id, [
                W.terminal(
                    lane.id, title: lane.title, activity: lane.state, line: lane.line,
                    turn: lane.state == "working" ? turn : nil, failed: lane.failed)
            ])
        }
        return try W.fleet([W.worktree("checkout", [W.terminal("orch", role: "orchestrator")])] + lanes)
    }

    static func rows() -> [[String: Any]] {
        NativeAgentSubagentTests.rows()
            + [T.row(20, "queued:1", ["Queued": ["text": "Also run the iOS UI tests for the band before you hand it back.", "state": "Waiting", "at_ms": 1]])]
    }

    /// The opened pane's stand-in: a terminal's paper.
    struct Shell: View {
        var body: some View {
            VStack(alignment: .leading, spacing: 4) {
                Text("~/Dev/overnight on main").foregroundStyle(.secondary)
                Text("$ ^C")
            }
            .font(.system(size: 12, design: .monospaced))
            .padding(Spacing.inset)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    /// The Agents panel as its popover draws it, for the capture.
    struct HeldPanel: View {
        let agents: RunningAgents
        var body: some View {
            AgentsPanel(agents: agents) { _ in }
                // Its own height, not the corner's it hangs from.
                .frame(height: 480, alignment: .top)
                .fixedSize()
                .glassEffect(.regular, in: .rect(cornerRadius: Radius.large))
                .shadow(color: .black.opacity(0.18), radius: 18, y: 6)
        }
    }

    struct Scene: View {
        let terminal: Terminal
        let native: NativeAgents
        let agents: RunningAgents
        let columns: Double
        let held: Bool
        let board: TaskBoardStore?

        var body: some View {
            let crumbs = [
                WorkspaceNavigation.Crumb(title: "Main", target: .workspace(host: "", workspace: W.main, focus: nil)),
                WorkspaceNavigation.Crumb(title: "overnight", target: .workspace(host: "", workspace: W.main, focus: nil), glyph: "arrow.triangle.branch"),
                WorkspaceNavigation.Crumb(title: "shell", target: nil, glyph: "apple.terminal"),
            ]
            WorkspaceView(
                opened: board == nil ? "shell" as String? : nil, hasConversation: true, hasBoard: false,
                cell: WorkspaceColumns.defaultCell, focused: false, navigatorWidth: .constant(320),
                conversation: {
                    NativeSwitch(terminal: terminal, target: "", isFocused: true, agents: native) { _ in Color.black }
                        .paneCard()
                        .overlay(alignment: .topTrailing) {
                            if held {
                                HeldPanel(agents: agents)
                                    .padding(.trailing, PaneCorner.inset)
                                    .offset(y: PaneCorner.inset + PaneCorner.control + 6)
                            }
                        }
                        .paneCanvas()
                        .environment(
                            \.paneCornerAccessory,
                            PaneCornerAccessory(terminal: terminal.id, content: AnyView(AgentsCornerButton(agents: agents) { _ in })))
                },
                navigator: { Color.clear },
                breadcrumb: { _ in DrillBreadcrumb(crumbs: crumbs, worktrees: nil, onGo: { _ in }, onClose: {}) },
                detail: { _, _ in Shell().paneCard().paneCanvas() },
                split: board == nil,
                chatColumns: .constant(columns),
                cards: { canvas in
                    guard !canvas, let board else { return AnyView(EmptyView()) }
                    return AnyView(WorkspaceStripRow { PlanStripRow(plan: board.plan, needsYou: 2, statuses: board.board.statuses) {} })
                })
            .toolbar {
                ToolbarItemGroup(placement: .navigation) {
                    Button {} label: { Image(systemName: "chevron.backward") }
                    Button {} label: { Image(systemName: "chevron.forward") }
                }
                ToolbarItemGroup(placement: .primaryAction) {
                    Button {} label: { Image(systemName: "plusminus") }.help("Show Changes")
                    Button {} label: { Image(systemName: "folder") }.help("Show Files")
                }
            }
        }
    }

    enum Layout: String { case narrow, wide, folded }

    static func shoot(_ layout: Layout, agents count: Int, held: Bool, variants: Int = 2) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        // Beside the canvas: the chat at its least and at its most, beside a
        // 560 pt canvas. Folded: a window too narrow for the canvas.
        let columns = Double(layout == .wide ? WorkspaceColumns.chatColumnsMaximum : WorkspaceColumns.chatColumnsMinimum)
        let chat = WorkspaceColumns.Canvas(navigator: 0, chatColumns: Int(columns), cell: WorkspaceColumns.defaultCell).chat
        let width = layout == .folded ? 760 : max(WorkspaceColumns.canvasFoldsBelow, (chat + WorkspaceColumns.divider + 560).rounded())
        let height: CGFloat = held ? 640 : 560
        let board = layout == .folded ? try await PlanSectionViewTests.store(.busy) : nil
        for (variant, appearance) in RealWindowCaptures.variants.prefix(variants) {
            let terminal = try T.terminal()
            let model = T.model(terminal)
            model.keys = Keys()
            model.store.apply(try await model.store.ledger.page(T.page(Self.rows())))
            let native = T.agents(model: model)
            model.showsNative = true
            let agents = MacAgents.running(workspace: W.main, host: "", fleet: try Self.fleet(agents: count))
            #expect(agents.count == count, "the fleet makes \(agents.count) agents, not \(count)")
            let reached = { @MainActor in
                model.offersSendNow && !model.store.shownIds.isEmpty && (board.map { PlanScenario.shown(.busy, in: $0.plan.plan) } ?? true)
            }
            let root = Scene(terminal: terminal, native: native, agents: agents, columns: columns, held: held, board: board)
                .frame(width: width, height: height)
                .background(WorkspaceStyle.canvas)
            let controller = NSHostingController(rootView: root)
            controller.sceneBridgingOptions = [.toolbars]
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height),
                styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.title = "Main"
            window.toolbarStyle = .unified
            window.contentViewController = controller
            window.setContentSize(NSSize(width: width, height: height))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            let expecting = "the conversation with Send Now offered"
            let name = "ov829-\(layout.rawValue)-\(count)\(held ? "-open" : "")-\(variant).png"
            try await Capture.keep(window, to: URL(fileURLWithPath: out).appendingPathComponent(name), expecting: expecting, until: reached)
            window.close()
            NativePaneModel.remember(false, for: terminal.id)
        }
    }

    @Test func atRest() async throws {
        for layout in [Layout.narrow, .wide, .folded] {
            for count in [1, 12] {
                try await Self.shoot(layout, agents: count, held: false)
            }
        }
    }

    @Test func open() async throws {
        try await Self.shoot(.narrow, agents: 12, held: true)
        try await Self.shoot(.folded, agents: 12, held: true)
    }

    /// Increased contrast, light and dark.
    @Test func contrast() async throws {
        try await Self.shoot(.narrow, agents: 12, held: false, variants: 4)
    }
}
