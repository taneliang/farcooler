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
    private func bar(scale: CGFloat? = nil) async throws -> (NSWindow, [String: CGRect]) {
        let terminal = TerminalCrumb(title: "server", siblings: [])
        let bar = DrillBreadcrumb(
            crumbs: [Nav.workspaceCrumb],
            worktrees: WorktreeCrumb(
                title: "invoice pdf", isHere: true, tasks: [], loose: [], target: .go(Nav.at(nil)), terminal: terminal),
            onGo: { _ in }, onClose: nil, menus: JumpMenuSource(count: 1, build: { [JumpMenu([])] }))
        let size = CGSize(width: 700, height: 60)
        let (window, seen) = try await {
            // SwiftUI rounds each view onto the pixel grid of its
            // `displayScale`, which a test sets for 1x (CI) or 2x.
            if let scale { return try await Nav.show(bar.environment(\.displayScale, scale), size: size) }
            return try await Nav.show(bar, size: size)
        }()
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

    /// The pieces' baselines: a 13 pt label and a 10 pt chevron, each
    /// centered in its cell, put the chevron's baseline above the label's,
    /// which no frame's center shows (review 1004o #6).
    static let baselineIDs = [
        "jump-label-0", "jump-caret-0", "jump-separator-2", "breadcrumb-worktrees", "jump-caret-1",
        "jump-separator-6", "breadcrumb-terminal", "jump-caret-2",
    ]

    @Test("Labels, carets and separators share one baseline, at 1x and 2x", arguments: [CGFloat(1), 2])
    func oneBaseline(scale: CGFloat) async throws {
        let (window, views) = try await bar(scale: scale)
        defer { window.close() }
        // Every piece aims at one whole-point baseline, so each rounds onto
        // the same pixel at 1x and 2x alike, and they are measured equal.
        // 0.5 pt is half a pixel at 1x: under the 1 pt the old centered
        // carets sat above the labels at 1x (2 pt for a separator), and the
        // 1.5 pt at 2x (`tolerance`).
        let baselines = try Self.baselineIDs.map { id in
            (id, try #require(views["\(id)-baseline"], "no baseline drawn for \(id): \(views.keys.sorted())").minY)
        }
        let first = baselines[0].1
        for (id, y) in baselines {
            #expect(abs(y - first) <= tolerance, "\(id)'s baseline is at \(y), the label's at \(first)")
        }
    }
}
