import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Two tiled pane cards on the window's plane (ov-221), the first focused: the
/// card's edge, its header and the way focus is said. Lost terminals, so no
/// process is started. A rendering, written by `VisualSpecimen`.
@MainActor
struct PaneCardSpecimenTests {
    @Test("Write the pane card sheet")
    func writeSheet() throws {
        try VisualSpecimen.shoot("pane-cards", size: CGSize(width: 900, height: 420), Self.tiles())
    }

    private static func tiles() -> TileView {
        let ids = ["a", "b"]
        let terminals = ids.map {
            Terminal(id: $0, short: $0, title: $0 == "a" ? "Orchestrator" : "Shell", preset: "claude", state: "LOST", epoch: 0)
        }
        let worktree = Worktree(
            id: "co", short: "co", task: "overnight", branch: "main", repository: "overnight", host: "",
            path: "/tmp/overnight", state: "active", terminals: terminals)
        let panes = ids.enumerated().map { index, id in
            PaneRect(
                id: id, short: id, title: nil, left: index * 61, top: 0, columns: 59, rows: 24,
                focused: index == 0, zoomed: false)
        }
        let group = PaneGroup(id: "@1", name: "", active: true, columns: 120, rows: 24, layout: "a", panes: panes)
        return TileView(
            groups: [group], showing: group.id, worktree: worktree,
            changes: ChangesStore(client: DaemonClient(target: ""), worktree: worktree),
            binary: nil, environment: [:], hostArguments: [], linkGeneration: 0, refusal: { nil },
            onFocus: { _ in }, onSelectGroup: { _ in }, onDropOnPane: { _, _, _ in },
            onViewport: { _, _, _ in }, onResizeDivider: { _, _, _ in true }, onSearchFiles: { _ in [] },
            onSwitchPaneMode: { _ in }, title: "Main", subtitle: "", setsTitle: false)
    }
}
