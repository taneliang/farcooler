import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// How far a pane's content starts from the window's top, measured in a
/// real titled window (ov-214, the owner: "this top area is pretty
/// inefficiently used").
///
/// Before: a 52 pt unified toolbar and its hairline, the conversation's
/// 32 pt header row and its divider, the canvas's 10 pt inset, then the lone
/// pane's own 28 pt title row, 124 pt in all (the design's §1, measured from
/// the owner's capture). After: the 52 pt regular toolbar, no header row,
/// the inset, and no title row on a lone pane: 62 pt.
///
/// The window is the harness's, with the real title bar, and the main area
/// is the real `TileView` the orchestrator's column draws, over a pane that
/// starts no process (a lost one, which draws its page). Where each pane's
/// header and body landed is read through `probed(_:)`.
@MainActor
@Suite(.serialized)
struct TopChromeTests {
    typealias Harness = TitleBarHarness

    /// Where the probed views landed, in the window, from its top.
    final class Seen {
        var views: [String: [CGRect]] = [:]
    }

    struct Probe<Content: View>: View {
        let seen: Seen
        let content: Content
        var body: some View {
            content
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let origin = proxy.frame(in: .global).origin
                        let _ = seen.views = Dictionary(grouping: probed, by: \.id).mapValues { views in
                            views.map { proxy[$0.bounds].offsetBy(dx: origin.x, dy: origin.y) }
                        }
                        Color.clear
                    }
                }
        }
    }

    private static func terminal(_ id: String) -> Terminal {
        Terminal(id: id, short: id, title: id, preset: "claude", state: "LOST", epoch: 0)
    }

    /// The orchestrator's column as the window draws it: one layout, `ids`
    /// stacked top to bottom.
    private static func tiles(_ ids: [String]) -> TileView {
        let terminals = ids.map(terminal)
        let worktree = Worktree(
            id: "co", short: "co", task: "overnight", branch: "main", repository: "overnight", host: "",
            path: "/tmp/overnight", state: "active", terminals: terminals)
        let rows = 48 / ids.count
        let panes = ids.enumerated().map { index, id in
            PaneRect(
                id: id, short: id, title: nil, left: 0, top: index * (rows + 1), columns: 120, rows: rows,
                focused: index == 0, zoomed: false)
        }
        let group = PaneGroup(id: "@1", name: "", active: true, columns: 120, rows: 49, layout: "a", panes: panes)
        return TileView(
            groups: [group], showing: group.id, worktree: worktree,
            changes: ChangesStore(client: DaemonClient(target: ""), worktree: worktree),
            binary: nil, environment: [:], hostArguments: [], linkGeneration: 0, refusal: { nil },
            onFocus: { _ in }, onSelectGroup: { _ in }, onDropOnPane: { _, _, _ in },
            onViewport: { _, _, _ in }, onResizeDivider: { _, _, _ in true }, onSearchFiles: { _ in [] },
            onSwitchPaneMode: { _ in }, title: "Main", subtitle: "", setsTitle: false)
    }

    /// The window with `ids` tiled under its title bar, and what landed where.
    private static func measure(_ ids: [String]) async throws -> (band: CGFloat, seen: [String: [CGRect]]) {
        let seen = Seen()
        let root = Harness.Root(
            words: Harness.Words(), content: Probe(seen: seen, content: tiles(ids)))
        let window = try await Harness.window(root, width: 1790, height: 800)
        defer { window.close() }
        return (Harness.band(window), seen.views)
    }

    @Test("A lone pane's content starts 62 pt below the window's top: the regular bar and the canvas inset")
    func lonePane() async throws {
        let (band, seen) = try await Self.measure(["orchestrator"])
        #expect(band == 52, "the toolbar is \(band) pt, not the regular 52")
        let body = try #require(seen["pane-body"]?.first, "no pane was drawn")
        #expect(abs(body.minY - 62) < 0.5, "the pane's content starts \(body.minY) pt down, not 62")
        #expect(seen["pane-header"] == nil, "a lone pane drew its title row")
    }

    @Test("Two panes each name themselves: the first's content is under its 28 pt title row")
    func twoPanes() async throws {
        let (band, seen) = try await Self.measure(["orchestrator", "shell"])
        #expect(band == 52, "the toolbar is \(band) pt, not the regular 52")
        let headers = try #require(seen["pane-header"])
        #expect(headers.count == 2, "\(headers.count) title rows for two panes")
        let top = try #require(seen["pane-body"]?.map(\.minY).min())
        #expect(abs(top - (62 + WorkspaceStyle.paneHeaderHeight)) < 0.5, "the first pane's content starts \(top) pt down")
    }
}
