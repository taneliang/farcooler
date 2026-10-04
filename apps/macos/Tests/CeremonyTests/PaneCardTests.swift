import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A pane card is opaque paper with no stroke, and its header says focus with
/// ink, not a wash (ov-221).
@MainActor
struct PaneCardTests {
    private func bitmap<V: View>(_ view: V, size: CGSize, _ name: NSAppearance.Name = .aqua) throws -> NSBitmapImageRep {
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
        host.appearance = NSAppearance(named: name)
        host.frame = CGRect(origin: .zero, size: size)
        host.layoutSubtreeIfNeeded()
        let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        return rep
    }

    private func color(_ rep: NSBitmapImageRep, _ x: Int, _ y: Int) throws -> NSColor {
        try #require(rep.colorAt(x: x * rep.pixelsWide / Int(rep.size.width), y: y * rep.pixelsHigh / Int(rep.size.height)))
            .usingColorSpace(.sRGB)!
    }

    @Test("A card's edge is its own color change: no stroke at standard contrast")
    func noStroke() throws {
        let size = CGSize(width: 80, height: 60)
        let rep = try bitmap(Color.clear.paneCard(), size: size)
        // On the left edge, and in the middle: the same paper.
        let edge = try color(rep, 0, 30)
        let middle = try color(rep, 40, 30)
        #expect(edge == middle, "something is drawn along the card's edge")
    }

    private static func tiles() -> TileView {
        let ids = ["a", "b"]
        let terminals = ids.map { Terminal(id: $0, short: $0, title: $0, preset: "claude", state: "LOST", epoch: 0) }
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

    @Test("The focused pane's header has no wash: it's the same color as another pane's")
    func focusIsNotAWash() throws {
        let size = CGSize(width: 900, height: 420)
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let rep = try bitmap(Self.tiles(), size: size, name)
            let y = Int(Pane.inset + WorkspaceStyle.paneHeaderHeight / 2)
            // Well clear of the number, the title and the status glyph.
            let focused = try color(rep, Int(Pane.inset) + 300, y)
            let other = try color(rep, 900 - Int(Pane.inset) - 100, y)
            #expect(focused == other, "the focused header is drawn differently in \(name.rawValue)")
        }
    }

    @Test("A card is Radius.medium")
    func radius() {
        #expect(Pane.radius == Radius.medium)
    }
}
