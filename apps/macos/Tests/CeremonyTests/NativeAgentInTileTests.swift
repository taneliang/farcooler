import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The conversation view in the real composition (ov-467): a claude pane in a
/// `TileView`, its native layer over the live terminal surface, in windows of
/// every height, so the grid's row remainder takes every value. The message
/// box must stay inside the pane's visible card, with the margin below it
/// that it has at its sides.
@MainActor
@Suite(.serialized)
struct NativeAgentInTileTests {
    typealias T = NativeAgentTests
    typealias P = NativePolishTests

    @Test("In a tiled pane the message box keeps its bottom margin at every window height")
    func theBoxStaysInsideTheVisibleCard() async throws { try await sweep(single: false) }

    @Test("In a lone pane, in a window with a toolbar, it keeps it too")
    func theBoxStaysInsideTheLonePane() async throws { try await sweep(single: true) }

    private func sweep(single: Bool) async throws {
        let id = "0199aaaa-0000-7000-8000-000000000001"
        let terminal = try T.terminal(id: id)
        let model = T.model(terminal)
        model.source = NativeAgentSubagentTests.Source(opens: true)
        model.store.apply(try await model.store.ledger.page(T.page(NativeAgentSubagentTests.rows())))
        model.showsNative = true
        model.rich = true
        let agents = NativeAgents.shared
        agents.pretend(enabled: true, rowsServed: true, core: RunnerCore())
        agents.adopt(model)
        defer { NativePaneModel.remember(false, for: id) }

        let worktree = Worktree(
            id: "co", short: "co", task: "overnight", branch: "main", repository: "overnight", host: "",
            path: "/tmp/overnight", state: "active", terminals: [terminal])
        for height in stride(from: 360, through: 560, by: 7) {
            for width in [CGFloat(480), 1_000] {
                let pane = PaneRect(id: id, short: "t1", title: nil, left: 0, top: 0, columns: 120, rows: 30, focused: true, zoomed: false)
                let group = PaneGroup(id: "@1", name: "", active: true, columns: 120, rows: 30, layout: "a", panes: [pane])
                let tiles = TileView(
                    groups: [group], showing: group.id, worktree: worktree,
                    changes: ChangesStore(client: DaemonClient(target: ""), worktree: worktree),
                    binary: nil, environment: [:], hostArguments: [], linkGeneration: 0,
                    onFocus: { _ in }, onSelectGroup: { _ in }, onDropOnPane: { _, _, _ in },
                    onViewport: { _, _, _ in }, onResizeDivider: { _, _, _ in true },
                    title: "Main", subtitle: "", setsTitle: false)
                let frames = P.Frames()
                let content: AnyView = single
                    ? AnyView(TerminalPane(
                        terminal: terminal, worktree: worktree, binary: nil, environment: [:], hostArguments: [], linkGeneration: 0,
                        onGeometry: { _, _ in }, onAction: { _ in }))
                    : AnyView(tiles.paneCanvas())
                let window = P.window(width: width, height: CGFloat(height), P.Framed(frames: frames, content: content))
                if single {
                    // The app's chrome: content under a transparent unified title bar and toolbar.
                    window.styleMask.insert(.fullSizeContentView)
                    window.titlebarAppearsTransparent = true
                    let toolbar = NSToolbar(identifier: "t")
                    window.toolbar = toolbar
                    window.toolbarStyle = .unified
                }
                await T.settle(window, still: frames)
                defer { window.close() }
                guard let box = frames.views["native-composer-box"], let body = single ? frames.views["native-agent-view"] : frames.views["pane-body"], let view = frames.views["native-agent-view"] else {
                    Issue.record("height \(height): nothing drawn: \(frames.views.keys.sorted())")
                    continue
                }
                let note = "window \(width)×\(height): box \(box), pane body \(body), native view \(view)"
                #expect(view.maxY <= (single ? CGFloat(height) - Pane.inset : body.maxY) + 0.5, "the view runs past the pane's bottom: \(note)")
                #expect(abs(body.maxY - box.maxY - Spacing.inset) < 1, "bottom margin is \(body.maxY - box.maxY), not \(Spacing.inset): \(note)")
            }
        }
    }
}
