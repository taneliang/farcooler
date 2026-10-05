import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The jump bar's pieces share one line and one rhythm (ov-290): a label, a
/// caret and a separator are each one cell tall, centered on the same y.
@MainActor
struct JumpBarAlignmentTests {
    typealias Nav = WorktreeTerminalNavigationTests

    /// A bar with every kind of piece: a linked crumb and its caret, the
    /// worktree label and its caret, the terminal and its caret, with a
    /// separator between each two.
    private func bar() async throws -> (NSWindow, [String: CGRect]) {
        let terminal = TerminalCrumb(title: "server", siblings: [])
        let bar = DrillBreadcrumb(
            crumbs: [Nav.workspaceCrumb],
            worktrees: WorktreeCrumb(
                title: "invoice pdf", isHere: true, tasks: [], loose: [], target: .go(Nav.at(nil)), terminal: terminal),
            onGo: { _ in }, onClose: nil, menus: JumpMenuSource(count: 1, build: { [JumpMenu([])] }))
        let (window, seen) = try await Nav.show(bar, size: CGSize(width: 700, height: 60))
        return (window, seen.views)
    }

    // Layout points, not pixels: SwiftUI lays out in points at 1x and 2x alike,
    // so 0.5 pt (a pixel at 2x, half of one at 1x) is far below the several
    // points the old baseline alignment put between a caret's center and its
    // neighbors'.
    private let tolerance: CGFloat = 0.5

    @Test("Labels, carets and separators share one vertical center")
    func oneCenter() async throws {
        let (window, views) = try await bar()
        defer { window.close() }
        let ids = [
            "jump-label-0", "jump-caret-0", "jump-separator-2", "breadcrumb-worktrees", "jump-caret-1",
            "jump-separator-6", "breadcrumb-terminal", "jump-caret-2",
        ]
        let centers = try ids.map { id in (id, try #require(views[id], "nothing drawn as \(id): \(views.keys.sorted())").midY) }
        let first = centers[0].1
        for (id, center) in centers { #expect(abs(center - first) <= tolerance, "\(id) is centered at \(center), not \(first)") }
    }

    @Test("Every caret is one size, and a caret sits against its separator with no gap of its own")
    func oneRhythm() async throws {
        let (window, views) = try await bar()
        defer { window.close() }
        let carets = try (0...2).map { try #require(views["jump-caret-\($0)"]) }
        for caret in carets {
            #expect(abs(caret.width - carets[0].width) <= tolerance && abs(caret.height - carets[0].height) <= tolerance)
            #expect(abs(caret.height - JumpBar.cell) <= tolerance, "a caret's hit area is \(caret.height) tall")
        }
        // Cells touch: the room between glyphs is the cells' own, the same
        // before every separator.
        let first = try #require(views["jump-separator-2"]), second = try #require(views["jump-separator-6"])
        #expect(abs(first.minX - carets[0].maxX) <= tolerance, "\(first.minX - carets[0].maxX) pt between a caret and its separator")
        #expect(abs(second.minX - carets[1].maxX) <= tolerance)
        #expect(abs(first.width - JumpBar.separatorWidth) <= tolerance)
    }
}
