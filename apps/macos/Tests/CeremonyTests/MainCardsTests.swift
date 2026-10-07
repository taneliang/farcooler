import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Every main-area destination sits on the one card (ov-297): the same paper,
/// radius and gutter a terminal and a task are drawn on, with the plane showing
/// around it. A destination that fills the area edge to edge fails here.
@MainActor
struct MainCardsTests {
    private static let size = CGSize(width: 700, height: 500)

    /// The real views of the destinations that stand alone, by name. The task
    /// and the tiled terminals are `contentCard()` and `paneCard()` at their
    /// call sites (`ContentView.openedView`, `TileView`); the source scan below
    /// covers those.
    private static func destinations() -> [(name: String, view: AnyView)] {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { _ in (nil, "unreachable") }
        let workspace = WorkspaceSummary.implicit(repository: "r")
        let defaults = UserDefaults(suiteName: "cards-\(UUID().uuidString)")!
        let plan = PlanStore(client: client, workspace: workspace, host: "mini", defaults: defaults)
        let context = PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })
        let worktree = Worktree(
            id: "w", short: "w", task: "overnight", branch: "main", repository: "r", host: "", path: "/tmp/w",
            state: "active", terminals: [])
        func needsYou(_ items: [NeedsYouItem]) -> AnyView {
            AnyView(
                NeedsYouView(
                    items: items, olderRunners: [], canAct: { _ in true }, onOpen: { _ in },
                    onAnswerAsk: { _, _ in nil }, onDecide: { _, _ in true }))
        }
        let item = NeedsYouItem(
            id: "ask:x", kind: .ask, rank: 1, since: nil, workspaceID: "ws", workspaceName: "Billing",
            repositoryID: "r", task: nil, terminal: nil, question: "Allow touch x?", askID: nil, actions: [])
        func fleet(_ phase: FleetPlaceholder.Phase) -> AnyView {
            AnyView(FleetPlaceholder(phase: phase, onAddRepository: {}, onNewWorktree: {}, onTryAgain: {}))
        }
        func conversation(_ state: ConversationColumn.State) -> AnyView {
            AnyView(
                ConversationPlaceholder(
                    state: state, offers: [], onStart: { _ in }, onRestart: {}, onReplace: {}))
        }
        var all: [(String, AnyView)] = [
            ("needs-you", needsYou([item])),
            ("needs-you-empty", needsYou([])),
            (
                "history",
                AnyView(
                    BoardHistoryView(
                        store: TaskBoardStore(client: client, workspace: workspace), status: .done, onOpen: { _ in }))
            ),
            (
                "plan-home",
                AnyView(PlanHome(board: TaskBoardStore(client: client, workspace: workspace), needsYou: PlanNeedsYou(), onOpen: { _ in }))
            ),
            ("plan-theme", AnyView(PlanPageView(plan: plan, page: .theme("t"), context: context))),
            ("plan-lane", AnyView(PlanPageView(plan: plan, page: .lane("l"), context: context))),
            ("plan-page", AnyView(PlanPageView(plan: plan, page: .page("p"), context: context))),
            (
                "worktree",
                AnyView(
                    WorktreeDetail(
                        worktree: worktree, onNewTerminal: {}, onHide: {}, onUnhide: {}, onRemove: {},
                        onOpenTerminal: { _ in }))
            ),
            ("no-orchestrator", WorkspaceMain.nothingOpen.eraseToAnyView()),
            (
                "files-inspector",
                AnyView(FilesInspector(routing: FilesRouting(), client: { _ in nil }, folderClient: { _ in nil }))
            ),
            ("orchestrator-none", conversation(.none)),
            ("orchestrator-starting", conversation(.starting(slow: false))),
            ("orchestrator-lost", conversation(.lost)),
            ("fleet-loading", fleet(.loading)),
            ("fleet-failed", fleet(.failed("boom"))),
            ("fleet-no-repositories", fleet(.noRepositories)),
            ("fleet-no-worktrees", fleet(.noWorktrees)),
            ("fleet-choose", fleet(.chooseWorkspace)),
            ("fleet-connecting", fleet(.connecting("mini"))),
            ("fleet-unreachable", fleet(.unreachable("mini", reason: "timed out"))),
        ]
        all.append(("card", AnyView(Color.clear.contentCard())))
        return all
    }

    private func bitmap(_ view: AnyView, _ name: NSAppearance.Name, scale: Int) throws -> NSBitmapImageRep {
        let host = NSHostingView(rootView: view.frame(width: Self.size.width, height: Self.size.height))
        host.appearance = NSAppearance(named: name)
        host.frame = CGRect(origin: .zero, size: Self.size)
        host.layoutSubtreeIfNeeded()
        // A turn of the run loop, for a view that reads before it draws.
        RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        host.layoutSubtreeIfNeeded()
        return try #require(host.lookBitmap(scale: scale))
    }

    /// The points that tell a card from a full-bleed view, in points: the
    /// window corner, the middle of the left edge and of the top (the gutter),
    /// the card's corner 1 pt in (clear because the corner is round: 12.7 pt
    /// from the arc's center against a radius of 10, a 2.7 pt margin that holds
    /// at 1x), and just inside the left edge, past the curve (paper).
    private static func probes() -> [(name: String, x: Int, y: Int)] {
        let gutter = Int(Pane.inset)
        return [
            ("window corner", 1, 1), ("left gutter", 1, 250), ("top gutter", 350, 1),
            ("card corner", gutter + 1, gutter + 1), ("card edge", gutter + 2, gutter + Int(Pane.radius) + 20),
        ]
    }

    /// Drawn on the window's plane, as the app draws it, so what is plane and
    /// what is paper can be told apart whatever the plane's own pixels are.
    private func onPlane(_ view: AnyView) -> AnyView {
        AnyView(view.background { WindowPlane() })
    }

    @Test("Every destination is paper inside the gutter and the plane outside it", arguments: lookScales)
    func everyDestinationIsACard(scale: Int) throws {
        var failures: [String] = []
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            // The card with nothing in it, on the plane: what each must match
            // at the probes, in the same appearance and scale.
            let reference = try bitmap(onPlane(AnyView(Color.clear.contentCard())), name, scale: scale)
            for (title, view) in Self.destinations() {
                let rep = try bitmap(onPlane(view), name, scale: scale)
                for probe in Self.probes() {
                    let want = reference.color(atPoint: probe.x, probe.y)
                    let got = rep.color(atPoint: probe.x, probe.y)
                    let apart = abs(want.redComponent - got.redComponent) + abs(want.greenComponent - got.greenComponent)
                        + abs(want.blueComponent - got.blueComponent) + abs(want.alphaComponent - got.alphaComponent)
                    // Anything visible is a different surface; 0.05 is
                    // rounding, which is the same at either scale.
                    if apart > 0.05 { failures.append("\(title) is not on the card at the \(probe.name) (\(name.rawValue), \(scale)x)") }
                }
            }
        }
        #expect(failures.isEmpty, "\(failures)")
    }

    @Test("The probes can tell a full-bleed view from a card")
    func probesFailFullBleed() throws {
        let reference = try bitmap(onPlane(AnyView(Color.clear.contentCard())), .aqua, scale: 1)
        let flat = try bitmap(onPlane(AnyView(Color.clear.background(WorkspaceStyle.paper))), .aqua, scale: 1)
        let differs = Self.probes().filter { probe in
            let a = reference.color(atPoint: probe.x, probe.y), b = flat.color(atPoint: probe.x, probe.y)
            return abs(a.redComponent - b.redComponent) + abs(a.greenComponent - b.greenComponent) + abs(a.blueComponent - b.blueComponent)
                + abs(a.alphaComponent - b.alphaComponent) > 0.05
        }
        #expect(differs.count >= 3, "a full-bleed view passes at all but \(differs.map(\.name))")
    }

    /// The source of the app's Swift, as the tests find it.
    private static func source(_ path: String) throws -> String {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("Sources/FarCooler")
        return try String(contentsOf: root.appendingPathComponent(path), encoding: .utf8)
    }

    @Test("What the main area draws for the task, the notices and the Files column is a card at its call site")
    func callSitesAreCards() throws {
        let detail = try Self.source("ContentView+WorkspaceDetail.swift")
        let opened = try #require(detail.range(of: "private func openedView"))
        let end = try #require(detail.range(of: "func goBack", range: opened.upperBound..<detail.endIndex))
        let body = String(detail[opened.lowerBound..<end.lowerBound])
        // Each arm that isn't a terminal (its own cards) or a view that draws
        // its own card says so.
        #expect(body.contains(".contentCard()"), "the task isn't a card")
        for notice in ["Board Not Found\", systemImage: \"checklist\"", "Board Not Found\", systemImage: \"map\"", "Worktree Not Found"] {
            let at = try #require(body.range(of: notice))
            #expect(body[at.upperBound...].prefix(80).contains(".contentCard()"), "\(notice) is full-bleed")
        }
        #expect(!body.contains("WorkspaceStyle.paper"), "an arm paints its own paper")
    }

    @Test("No view paints paper over the whole main area: paper is only ever inside a card")
    func noFullBleedPaper() throws {
        // Files whose `.background(WorkspaceStyle.paper)` is inside a card: a
        // pane of a tile, a task tab, or the Files column.
        let inside: Set<String> = [
            "ContentView+WorkspaceDetail.swift",  // the task's overview tab
            "AgentSurface.swift", "ChangesPane.swift", "Files/CodeView.swift", "Files/FilesPane.swift",
            // A pane's native view, in the pane's card as the chat is (ov-372).
            "NativeAgent/NativeAgentView.swift",
        ]
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("Sources/FarCooler")
        let walker = try #require(FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil))
        // Paper as a surface's fill, by count: the card itself, and the plan
        // peeked over the chat, a card that floats over a card. Anything
        // else is a card drawn by hand beside `contentCard()`, which a change
        // to its gutter or radius would leave behind (train 1004r, M1).
        let fills: [String: Int] = ["PaneCanvas.swift": 1, "Plan/PlanHome.swift": 1]
        var offenders: [String] = []
        for case let url as URL in walker where url.pathExtension == "swift" {
            let relative = String(url.path.dropFirst(root.path.count + 1))
            let text = try String(contentsOf: url, encoding: .utf8)
            if text.contains(".background(WorkspaceStyle.paper)"), !inside.contains(relative) { offenders.append(relative) }
            let filled = text.components(separatedBy: "fill: WorkspaceStyle.paper").count - 1
            if filled > fills[relative, default: 0] { offenders.append("\(relative), a card by hand") }
        }
        #expect(offenders.isEmpty, "paper behind a whole view in \(offenders)")
    }
}

private extension View {
    func eraseToAnyView() -> AnyView { AnyView(self) }
}
