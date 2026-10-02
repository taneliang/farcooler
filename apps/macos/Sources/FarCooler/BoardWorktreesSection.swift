import AgentKit
import SwiftUI

/// What the board list knows of its workspace's worktrees (ov-86): the one
/// each task's row names, and the loose ones its Worktrees section lists.
/// A value handed to the board by the window, which holds the fleet and
/// the selection.
struct BoardWorktrees {
    /// Each task's worktree, by task id: named beside its key, with its
    /// menu on the task's.
    var byTask: [String: Worktree] = [:]
    /// The Worktrees section: the loose ones (`WorkspaceWorktrees.loose`).
    var shown: [Worktree] = []
    var hidden: [Worktree] = []
    /// The worktree open whole beside the board, drawn selected.
    var selected: String?
    var onOpen: (Worktree) -> Void = { _ in }
    /// New Worktree…, the section header's +. Nil where the runner can't
    /// take one.
    var onNew: (() -> Void)?
    var onUnhide: ((Worktree) -> Void)?
    /// A worktree's menu, the sidebar row's (`WorktreeMenu.items`), and
    /// what choosing an item does.
    var menu: (Worktree) -> [WorktreeMenu.Item] = { _ in [] }
    var perform: (WorktreeMenu.Item, Worktree) -> Void = { _, _ in }
    /// Where the section's collapsed state is kept, per board.
    var collapseKey = ""

    static var none: BoardWorktrees { BoardWorktrees() }

    /// Whether the board draws the section at all: not for a board whose
    /// window has no fleet to give it.
    var isEmpty: Bool { shown.isEmpty && hidden.isEmpty && onNew == nil }
}

/// The Worktrees section at the bottom of the board list (ov-86): the
/// worktrees no task has, the main checkout and scratch ones, each opened
/// beside the board on a click, with the hidden ones collapsed under them
/// and New Worktree… on the header's +.
///
/// On the board's grid, as a status section is: its chevron at column A,
/// its title at B, its count trailing; its rows' branch glyph at B and
/// names at C.
struct BoardWorktreesSection: View {
    let worktrees: BoardWorktrees
    /// The list has the keyboard: a selected row reads in the accent.
    let keyed: Bool

    @State private var collapsed: Bool
    @State private var hiddenExpanded = false
    private let defaults: UserDefaults

    init(worktrees: BoardWorktrees, keyed: Bool, defaults: UserDefaults = .standard) {
        self.worktrees = worktrees
        self.keyed = keyed
        self.defaults = defaults
        _collapsed = State(initialValue: defaults.bool(forKey: worktrees.collapseKey))
    }

    private var expanded: Bool { !collapsed }

    var body: some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
            HStack(spacing: 0) {
                Button {
                    withAnimation(Motion.snap) { collapsed.toggle() }
                    if !worktrees.collapseKey.isEmpty { defaults.set(collapsed, forKey: worktrees.collapseKey) }
                } label: {
                    HStack(spacing: 0) {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .bold))
                            .rotationEffect(.degrees(expanded ? 90 : 0))
                            .foregroundStyle(.secondary)
                            .frame(width: ColumnGrid.step, alignment: .leading)
                            .gridMark("worktrees", .chevron)
                        Text("Worktrees")
                            .font(WorkspaceStyle.sectionTitle)
                            .foregroundStyle(worktrees.shown.isEmpty ? Color.secondary : Color.primary)
                            .gridMark("worktrees", .text)
                        Spacer(minLength: SidebarGrid.gap)
                    }
                    .frame(minHeight: ColumnGrid.rowHeight)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Worktrees, \(worktrees.shown.count)")
                .accessibilityValue(expanded ? "Expanded" : "Collapsed")
                .accessibilityIdentifier("board-worktrees")
                if let onNew = worktrees.onNew {
                    Button(action: onNew) {
                        Image(systemName: "plus")
                            .font(.system(size: 11, weight: .medium))
                            .frame(width: SidebarGrid.control, height: SidebarGrid.control)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.borderless)
                    .padding(.trailing, SidebarGrid.gap)
                    .help("New Worktree…")
                    .accessibilityLabel("New Worktree")
                    .accessibilityIdentifier("board-new-worktree")
                }
                // Trailing, under the status sections' counts.
                Text("\(worktrees.shown.count)")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
            }
            if expanded {
                ForEach(worktrees.shown) { worktree in
                    BoardWorktreeRow(
                        worktree: worktree, selected: worktree.id == worktrees.selected, keyed: keyed,
                        onOpen: { worktrees.onOpen(worktree) },
                        menu: worktrees.menu(worktree), perform: { worktrees.perform($0, worktree) })
                }
                if !worktrees.hidden.isEmpty {
                    hiddenGroup
                }
            }
        }
    }

    /// The hidden ones, collapsed under a "Hidden 2" line, each with
    /// Unhide, as the sidebar keeps them.
    private var hiddenGroup: some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm / 2) {
            Button {
                withAnimation(Motion.snap) { hiddenExpanded.toggle() }
            } label: {
                HStack(spacing: 0) {
                    Image(systemName: "eye.slash")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .frame(width: ColumnGrid.step, alignment: .leading)
                    Text("Hidden")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                    Spacer(minLength: SidebarGrid.gap)
                    Text("\(worktrees.hidden.count)")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
                .padding(.leading, ColumnGrid.step)
                .frame(minHeight: ColumnGrid.rowHeight)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Hidden, \(worktrees.hidden.count)")
            .accessibilityValue(hiddenExpanded ? "Expanded" : "Collapsed")
            if hiddenExpanded {
                ForEach(worktrees.hidden) { worktree in
                    HStack(spacing: SidebarGrid.gap) {
                        Text(worktree.task)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                        Spacer(minLength: 0)
                        if let unhide = worktrees.onUnhide {
                            Button("Unhide") { unhide(worktree) }
                                .buttonStyle(.plain)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(.secondary)
                        }
                    }
                    .padding(.leading, 2 * ColumnGrid.step)
                    .frame(minHeight: ColumnGrid.rowHeight)
                }
            }
        }
    }
}

/// One loose worktree in the board list: its branch glyph and name, its
/// branch beneath, and what's waiting in it.
private struct BoardWorktreeRow: View {
    let worktree: Worktree
    let selected: Bool
    let keyed: Bool
    let onOpen: () -> Void
    let menu: [WorktreeMenu.Item]
    let perform: (WorktreeMenu.Item) -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        // A button, so it's reached by the keyboard as well as the mouse.
        Button(action: onOpen) { label }
            .buttonStyle(.plain)
            .contextMenu { WorktreeMenuItems(items: menu, perform: perform) }
            .accessibilityAddTraits(selected ? .isSelected : [])
            .accessibilityIdentifier("board-worktree-\(worktree.task)")
    }

    private var label: some View {
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            Image(systemName: WorktreeSection.glyph)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(width: ColumnGrid.step, alignment: .leading)
                .gridMark("boardWorktree", .icon)
            VStack(alignment: .leading, spacing: 2) {
                Text(worktree.isMainCheckout ? "\(worktree.task) (main checkout)" : worktree.task)
                    .font(.system(size: WorkspaceStyle.PaneText.body))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .gridMark("boardWorktree", .text)
                Text(worktree.summary)
                    .font(.system(size: WorkspaceStyle.PaneText.minimum))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: SidebarGrid.gap)
            if let status = worktree.attentionStatus {
                Circle()
                    .fill(status.wantsAttention ? GlancePalette.amber(scheme) : Color.secondary)
                    .frame(width: 7, height: 7)
                    .help(status.label)
            }
        }
        .padding(.horizontal, ColumnGrid.step)
        .padding(.vertical, ColumnGrid.rhythm / 2)
        .frame(minHeight: ColumnGrid.twoLineRowHeight)
        .background {
            if selected {
                RoundedRectangle(cornerRadius: 8)
                    .fill(keyed ? Color.accentColor.opacity(0.18) : Color.primary.opacity(0.08))
            }
        }
        .contentShape(Rectangle())
    }
}

/// A worktree's menu items (`WorktreeMenu.items`), drawn as the sidebar
/// row's menus draw them: Move to Workspace folded into a submenu, the
/// hiding and removing below a divider.
struct WorktreeMenuItems: View {
    let items: [WorktreeMenu.Item]
    let perform: (WorktreeMenu.Item) -> Void

    var body: some View {
        let moves = items.filter { if case .move = $0 { return true } else { return false } }
        ForEach(items.filter { [.open, .showChanges, .newTerminal].contains($0) }, id: \.self) { item in
            Button(item.title) { perform(item) }
        }
        if !moves.isEmpty {
            Menu("Move to Workspace") {
                ForEach(moves, id: \.self) { item in Button(item.title) { perform(item) } }
            }
        }
        ForEach(items.filter { if case .useAsOrchestrator = $0 { return true } else { return false } }, id: \.self) { item in
            Button(item.title) { perform(item) }
        }
        let last = items.filter { [.hide, .unhide, .remove, .dismiss].contains($0) }
        if !last.isEmpty {
            Divider()
            ForEach(last, id: \.self) { item in
                Button(item.title, role: item == .remove ? .destructive : nil) { perform(item) }
            }
        }
    }
}
