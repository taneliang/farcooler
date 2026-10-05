import AgentKit
import AppKit
import SwiftUI

// A jump bar segment is two controls (ov-267): its label, which goes to what
// it names, and its caret, which opens the menu of what's beside it. One bar
// then moves up the hierarchy, by a label, and sideways, by a caret. The
// owner, 4 October: clicking the label navigates; only the caret opens the
// dropdown.

/// The terminal a worktree's segment is followed by, where one is open in
/// it: the bar's last segment, a menu of the worktree's other terminals.
struct TerminalCrumb: Equatable {
    var title: String
    /// The worktree's terminals, the open one checked
    /// (`JumpMenus.terminals`).
    var siblings: [JumpSection]

    var jumpMenu: JumpMenu { JumpMenu(siblings) }

    /// The segment for terminal `id` in `worktree`, or nil for one that
    /// isn't among its own.
    static func of(_ id: String?, in worktree: Worktree, selection: ContentView.Selection?, fleet: Fleet)
        -> TerminalCrumb?
    {
        guard let id, let terminal = worktree.terminals.first(where: { $0.id == id }), !terminal.isOrchestrator
        else { return nil }
        let sections = JumpMenus.terminals(of: worktree, selection: selection, fleet: fleet).map { section in
            JumpSection(
                title: section.title,
                items: section.items.map { item in
                    var item = item
                    if item.id == "term|\(id)" { item.current = true }
                    return item
                }, more: section.more)
        }
        return TerminalCrumb(title: worktree.name(of: terminal), siblings: sections)
    }
}

extension DrillBreadcrumb {
    /// What a click on one of the bar's pieces does.
    enum Click: Equatable {
        /// Go there: a label's.
        case go(JumpTarget)
        /// Open segment `n`'s menu: a caret's, or the label of a segment
        /// that names no one place ("Worktrees" beside a task with several).
        case menu(Int)
        /// Nothing: the label of where you are.
        case none
    }

    /// The click `piece` answers, in a bar of `crumbs` whose first `menus`
    /// have a menu, with `worktrees` after them.
    static func click(_ piece: Piece.Kind, crumbs: [WorkspaceNavigation.Crumb], worktrees: WorktreeCrumb?) -> Click {
        switch piece {
        case .separator, .menuTitle:
            return .none
        case .crumb(let index):
            return crumbs[index].target.map { .go(.go($0)) } ?? .none
        case .crumbCaret(let index):
            return .menu(index)
        case .menuIcon:
            guard let worktrees else { return .none }
            if let opens = worktrees.opens {
                return .go(opens.trail.map { .open(opens.target, from: $0) } ?? .go(opens.target))
            }
            if let target = worktrees.target { return .go(target) }
            return worktrees.isHere ? .none : .menu(crumbs.count)
        case .menuChevron:
            return .menu(crumbs.count)
        case .terminalTitle:
            return .none
        case .terminalChevron:
            return .menu(crumbs.count + 1)
        }
    }

    /// What VoiceOver calls a label: where it goes.
    static func labelName(_ title: String) -> String { "Go to \(title)" }

    /// What VoiceOver calls segment `index`'s caret, in a bar of `crumbs`
    /// crumbs: what its menu holds. A middle crumb's names `title`, its
    /// segment, so two carets in one bar don't read alike.
    static func caretName(_ index: Int, crumbs: Int, title: String? = nil) -> String {
        if index == 0, crumbs > 0 { return "Show other workspaces" }
        if index < crumbs { return title.map { "Show places beside \($0)" } ?? "Show other places in this workspace" }
        if index == crumbs { return "Show other worktrees" }
        return "Show other terminals"
    }
}

/// A capture's pointer (review 1004o #5, #7): the jump bar's labels and
/// carets whose `name` (what VoiceOver calls them) is here draw their hover
/// backing as if the pointer rested on them, with no input sent. Set only by
/// tests and `RealWindowCaptures` (`FARCOOLER_CAPTURE_HOVER`); empty in the
/// app, and read when a piece is drawn.
@MainActor
enum JumpBarHover {
    static var forced: Set<String> = []
}

/// A segment's label as a control: its own hover highlight, apart from the
/// caret's.
struct JumpLabelButton<Label: View>: View {
    let name: String
    let action: () -> Void
    @ViewBuilder let label: () -> Label

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            label()
                .padding(.horizontal, JumpBar.labelInset)
                .jumpCell()
                .background(RoundedRectangle.control.fill(hovering || JumpBarHover.forced.contains(name) ? Fill.hover : Color.clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(name)
        .accessibilityLabel(name)
    }
}

/// A segment's caret: at least `minWidth` wide and `minHeight` tall to hit,
/// with its own hover highlight, which is that hit area and is drawn only on
/// hover, so it never moves the layout. A label is as tall to hit.
struct JumpCaretButton: View {
    let name: String
    let style: JumpBar.Style
    /// Where a test reads its glyph's baseline (`baselineProbed`).
    var probe: String? = nil
    let action: () -> Void

    /// The narrowest a caret's hit area is (ov-267).
    static let minWidth: CGFloat = 20

    /// The shortest a caret's or a label's hit area is (ov-267 review, ov-290):
    /// one cell, which every piece of the bar is, so they share a center.
    static let minHeight: CGFloat = JumpBar.cell

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: JumpBar.menuGlyph)
                .font(style.font)
                .foregroundStyle(style.color)
                .baselineProbed(probe)
                .jumpCell()
                .frame(minWidth: Self.minWidth, minHeight: Self.minHeight)
                .background(RoundedRectangle.control.fill(hovering || JumpBarHover.forced.contains(name) ? Fill.hover : Color.clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(name)
        .accessibilityLabel(name)
    }
}

/// Where every piece's baseline sits in its cell (ov-290, review 1004o #6):
/// a label's, as it falls when the text is centered in the cell, so a
/// smaller chevron rides the label's baseline rather than centering itself
/// above it. Every cell is still `JumpBar.cell` tall, on one center. A
/// whole point, so each piece rounds onto the same pixel at 1x as at 2x.
extension JumpBar {
    static let baseline: CGFloat = {
        let font = NSFont.systemFont(ofSize: ColumnHeader.textSize)
        return (cell / 2 + (font.ascender + font.descender) / 2).rounded()
    }()
}

extension VerticalAlignment {
    private enum JumpBaseline: AlignmentID {
        static func defaultValue(in d: ViewDimensions) -> CGFloat { JumpBar.baseline }
    }

    /// A jump bar cell's baseline (`JumpBar.baseline`), from its top.
    static let jumpBaseline = VerticalAlignment(JumpBaseline.self)
}

extension View {
    /// This piece in a jump bar cell: `JumpBar.cell` tall, its first text
    /// baseline at `JumpBar.baseline`, centered across.
    func jumpCell() -> some View {
        ZStack(alignment: Alignment(horizontal: .center, vertical: .jumpBaseline)) {
            Color.clear.frame(width: 0, height: JumpBar.cell)
            alignmentGuide(.jumpBaseline) { $0[.firstTextBaseline] }
        }
    }
}
