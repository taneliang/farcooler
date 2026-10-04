import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Every column header in a workspace is one height over one divider, so
/// their bottom edges are one line across the window (ov-92: the board's
/// "Main" header stood taller than the orchestrator's), and the jump bar's
/// segments are one size (owner, 2 Oct: "Main" small, the task large and
/// bold, "Worktrees" a third size). The board's and the orchestrator's
/// headers went in ov-214; the jump bar's, over a task or a worktree, are
/// the ones left.
@MainActor
struct ColumnHeaderTests {
    private func height<V: View>(_ view: V, width: CGFloat = 640) -> CGFloat {
        NSHostingController(rootView: view).sizeThatFits(in: CGSize(width: width, height: 400)).height
    }

    private static let longTask = WorkspaceNavigation.Crumb(
        title: "ov-6 Tunnel migration UI for existing runners (parked) and a good deal more besides", target: nil)
    private static let main = WorkspaceNavigation.Crumb(
        title: "Main", target: .workspace(host: "", workspace: "ws", focus: nil))
    private static let menu = WorktreeCrumb(title: "Worktrees", isHere: false, tasks: [], loose: [])

    @Test("Every column header is the shared height over the one divider", arguments: [280, 640, 1200] as [CGFloat])
    func everyHeaderIsOneHeight(width: CGFloat) {
        let headers: [(String, CGFloat)] = [
            ("jump bar", height(
                DrillBreadcrumb(crumbs: [Self.main, Self.longTask], worktrees: Self.menu, onGo: { _ in }, onClose: {}),
                width: width)),
            ("jump bar, a worktree", height(
                DrillBreadcrumb(
                    crumbs: [Self.main],
                    worktrees: WorktreeCrumb(title: "tax-rounding", isHere: true, tasks: [], loose: []),
                    onGo: { _ in }, onClose: nil),
                width: width)),
        ]
        for (name, measured) in headers {
            #expect(measured == ColumnHeader.total, "\(name) is \(measured) pt at \(width), not \(ColumnHeader.total)")
        }
    }

    @Test("Every jump bar segment is the header's one size, ancestors secondary, the current one primary")
    func jumpBarTypography() {
        let cases: [([WorkspaceNavigation.Crumb], WorktreeCrumb?)] = [
            ([Self.main, Self.longTask], Self.menu),
            ([Self.main], WorktreeCrumb(title: "tax-rounding", isHere: true, tasks: [], loose: [])),
            ([Self.main, Self.longTask], nil),
        ]
        for (crumbs, worktrees) in cases {
            let pieces = DrillBreadcrumb.pieces(crumbs, worktrees: worktrees)
            #expect(!pieces.isEmpty)
            for piece in pieces {
                switch piece.kind {
                case .separator, .menuChevron, .crumbCaret, .terminalChevron:
                    // One chevron, one size and weight, tertiary.
                    #expect(piece.style == JumpBar.chevron, "\(piece.kind)")
                case .crumb(let index):
                    #expect(piece.style.size == ColumnHeader.textSize)
                    if crumbs[index].target == nil {
                        #expect(piece.style.tone == .primary)
                    } else {
                        #expect(piece.style.tone == .secondary && piece.style.weight == .regular)
                    }
                case .terminalTitle:
                    #expect(piece.style == JumpBar.style(.current))
                case .menuIcon, .menuTitle:
                    #expect(piece.style.size == ColumnHeader.textSize, "\(piece.kind)")
                    #expect(piece.style.tone == (worktrees?.isHere == true ? .primary : .secondary))
                }
            }
            // The icon is drawn as its title is.
            let icon = pieces.first { $0.kind == .menuIcon }?.style
            let title = pieces.first { $0.kind == .menuTitle }?.style
            #expect(icon == title)
            // One segment is where you are: the current crumb, or the
            // worktree menu standing for it, its icon and title.
            #expect(pieces.filter { $0.style.tone == .primary }.count == (worktrees?.isHere == true ? 2 : 1))
        }
    }
}
