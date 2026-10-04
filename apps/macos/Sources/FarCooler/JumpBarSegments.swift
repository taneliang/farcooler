import AgentKit
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
                .padding(.horizontal, 3)
                .frame(minHeight: JumpCaretButton.minHeight)
                .background(RoundedRectangle.control.fill(hovering ? Fill.hover : Color.clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(name)
        .accessibilityLabel(name)
    }
}

/// A segment's caret: at least `minWidth` wide and `minHeight` tall to hit,
/// with its own hover highlight. A label is as tall to hit.
struct JumpCaretButton: View {
    let name: String
    let style: JumpBar.Style
    let action: () -> Void

    /// The narrowest a caret's hit area is (ov-267).
    static let minWidth: CGFloat = 20

    /// The shortest a caret's or a label's hit area is (ov-267 review): a
    /// label's own text is 13 to 15 pt, too small a target beside a caret.
    static let minHeight: CGFloat = 28

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: JumpBar.menuGlyph)
                .font(style.font)
                .foregroundStyle(style.color)
                .frame(minWidth: Self.minWidth, minHeight: Self.minHeight)
                .background(RoundedRectangle.control.fill(hovering ? Fill.hover : Color.clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(name)
        .accessibilityLabel(name)
    }
}
