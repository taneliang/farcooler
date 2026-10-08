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
    /// Each shown worktree's own terminals, by worktree id, listed under its
    /// row (ov-267): where a terminal opened in it lives.
    var worktreeTerminals: [String: [Terminal]] = [:]
    /// The terminal open in the main area, by id, drawn selected under its
    /// worktree in place of the worktree's row.
    var selectedTerminal: String?
    var onOpenTerminal: (Worktree, Terminal) -> Void = { _, _ in }
    /// New Worktree…, the section's trailing row. Nil where the runner
    /// can't take one.
    var onNew: (() -> Void)?
    var onUnhide: ((Worktree) -> Void)?
    /// A worktree's menu, the sidebar row's (`WorktreeMenu.items`), and
    /// what choosing an item does.
    var menu: (Worktree) -> [WorktreeMenu.Item] = { _ in [] }
    var perform: (WorktreeMenu.Item, Worktree) -> Void = { _, _ in }
    /// The repository's own terminals, in their main checkout: the
    /// Terminals section beside the tasks (ov-178).
    var terminals: ProjectTerminals = .none
    /// The shown ones no workspace owns, by id: Main lists its
    /// repository's, captioned Unclaimed (ov-178, where the old sidebar's
    /// Unclaimed group went), so Move to Workspace has its prompt.
    var unclaimed: Set<String> = []

    static var none: BoardWorktrees { BoardWorktrees() }

    /// The terminals listed under `worktree`'s row: its own, not an
    /// orchestrator seated in it, which only its conversation shows.
    static func terminals(of worktree: Worktree, in fleet: Fleet) -> [Terminal] {
        WorkspaceScreen.ownTerminals(of: worktree, fleet: fleet).terminals.filter { !$0.isOrchestrator }
    }

    /// Whether `worktree`'s own row is drawn selected: open whole, not one
    /// of its terminals listed under it.
    func rowSelected(_ worktree: Worktree) -> Bool {
        worktree.id == selected && !(worktreeTerminals[worktree.id] ?? []).contains { $0.id == selectedTerminal }
    }

    /// Which of `worktrees` no workspace on their runner owns
    /// (`WorkspaceSelection.owner`). Never the main checkout, which is the
    /// repository's own directory rather than a worktree waiting to be
    /// claimed; and none on a runner without workspaces, where there's
    /// nothing to claim for.
    static func unclaimed(_ worktrees: [Worktree], in fleet: Fleet) -> Set<String> {
        Set(
            worktrees.filter { worktree in
                !worktree.isMainCheckout && fleet.runnerWorkspaces[worktree.host ?? ""] != nil
                    && WorkspaceSelection.owner(of: worktree, in: fleet) == nil
            }.map(\.id))
    }

    /// A loose worktree row's second line: what's in it, after "Unclaimed"
    /// for one no workspace owns.
    static func caption(_ worktree: Worktree, unclaimed: Bool) -> String {
        unclaimed ? "Unclaimed · \(worktree.summary)" : worktree.summary
    }

    /// Whether the board draws the section at all: not for a board whose
    /// window has no fleet to give it.
    var isEmpty: Bool { shown.isEmpty && hidden.isEmpty && onNew == nil }
}

/// The Worktrees section at the bottom of the navigator (ov-86, ov-92): the
/// worktrees no task has, the main checkout and scratch ones, each opened
/// in the main area on a click, with the hidden ones collapsed under them,
/// and New Worktree… trailing. Its header is the navigator's
/// (`CollapsibleSection`, in the navigator style), apart from the tasks' status groups.
///
/// On the navigator's grid: its rows' branch glyph at B and names at C.
struct BoardWorktreesSection: View {
    let worktrees: BoardWorktrees
    /// The list has the keyboard: a selected row reads in the accent.
    let keyed: Bool

    @State private var hiddenExpanded = false

    init(worktrees: BoardWorktrees, keyed: Bool) {
        self.worktrees = worktrees
        self.keyed = keyed
    }

    /// The rows ↑ and ↓ walk here: the shown ones, in order.
    static func rows(_ worktrees: BoardWorktrees) -> [Worktree] { worktrees.shown }

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.row) {
            ForEach(Self.rows(worktrees)) { worktree in
                BoardWorktreeRow(
                    worktree: worktree, selected: worktrees.rowSelected(worktree), keyed: keyed,
                    caption: BoardWorktrees.caption(worktree, unclaimed: worktrees.unclaimed.contains(worktree.id)),
                    onOpen: { worktrees.onOpen(worktree) },
                    menu: worktrees.menu(worktree), perform: { worktrees.perform($0, worktree) })
                .id(NavigatorItem.worktree(worktree.id))
                ForEach(worktrees.worktreeTerminals[worktree.id] ?? []) { terminal in
                    BoardWorktreeTerminalRow(
                        terminal: terminal, name: worktree.name(of: terminal), selected: terminal.id == worktrees.selectedTerminal, keyed: keyed,
                        onOpen: { worktrees.onOpenTerminal(worktree, terminal) })
                }
            }
            if !worktrees.hidden.isEmpty {
                hiddenGroup
                    .padding(.top, NavigatorRhythm.subgroup)
            }
            if let onNew = worktrees.onNew {
                Button(action: onNew) {
                    HStack(spacing: 0) {
                        Image(systemName: "plus")
                            .font(.system(size: 10, weight: .medium))
                            .gridMark("boardWorktreeNew", .icon)
                            .glyphColumn()
                        Text("New Worktree…")
                            .font(.system(size: WorkspaceStyle.PaneText.body))
                        Spacer(minLength: 0)
                    }
                    .foregroundStyle(SidebarInk.secondary)
                    .padding(.vertical, NavigatorRhythm.air)
                    .probed("board-new-worktree-line")
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("New Worktree…")
                .accessibilityLabel("New Worktree")
                .accessibilityIdentifier("board-new-worktree")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-worktrees")
    }

    /// The hidden ones, collapsed under a "Hidden 2" line, each with
    /// Unhide, as the sidebar keeps them.
    private var hiddenGroup: some View {
        CollapsibleSection(
            "Hidden", id: "worktrees.hidden", style: .minor, isExpanded: $hiddenExpanded,
            count: worktrees.hidden.count
        ) {
            VStack(alignment: .leading, spacing: NavigatorRhythm.row) {
                ForEach(worktrees.hidden) { worktree in
                    HStack(spacing: SidebarGrid.gap) {
                        Text(worktree.task)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(SidebarInk.secondary)
                            .lineLimit(1)
                        Spacer(minLength: 0)
                        if let unhide = worktrees.onUnhide {
                            Button("Unhide") { unhide(worktree) }
                                .buttonStyle(.plain)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(SidebarInk.secondary)
                        }
                    }
                    .padding(.leading, NavigatorGrid.textInset)
                    .padding(.vertical, NavigatorRhythm.air)
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
    /// The line under its name (`BoardWorktrees.caption`).
    let caption: String
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
                .foregroundStyle(SidebarInk.secondary)
                .gridMark("boardWorktree", .icon)
                .glyphColumn()
            VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                Text(worktree.isMainCheckout ? "\(worktree.task) (main checkout)" : worktree.task)
                    .font(.system(size: WorkspaceStyle.PaneText.body))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .gridMark("boardWorktree", .text)
                Text(caption)
                    .font(.system(size: WorkspaceStyle.PaneText.minimum))
                    .foregroundStyle(SidebarInk.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: SidebarGrid.gap)
            if let status = worktree.attentionStatus {
                Circle()
                    // The status table's ink (ov-137): a failed agent is red,
                    // not the amber that means it needs you.
                    .fill(status.tone.color(scheme))
                    .frame(width: 7, height: 7)
                    .gridMark("boardWorktree", .trailing)
                    .help(status.label)
            }
        }
        .navigatorRow(selected: selected, keyed: keyed, leading: 0, box: "boardWorktree")
    }
}

/// One of a loose worktree's terminals, under its row (ov-267): its glyph
/// and name at the worktree's text column, and its status.
private struct BoardWorktreeTerminalRow: View {
    let terminal: Terminal
    /// Its label, numbered from the second alike (`Worktree.name(of:)`).
    let name: String
    let selected: Bool
    let keyed: Bool
    let onOpen: () -> Void

    var body: some View {
        Button(action: onOpen) {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                Image(systemName: terminal.glyph)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(SidebarInk.secondary)
                    .glyphColumn()
                Text(name)
                    .font(.system(size: WorkspaceStyle.PaneText.body))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .identified("board-worktree-terminal-name-\(terminal.id)")
                Spacer(minLength: SidebarGrid.gap)
                StatusGlyph(status: terminal.status)
                    .help(terminal.status.label)
            }
            .navigatorRow(selected: selected, keyed: keyed)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(name)
        .accessibilityValue(terminal.status.label)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .identified("board-worktree-terminal-\(terminal.id)")
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
        ForEach(items.filter { [.open, .openInEditor, .showChanges, .newTerminal].contains($0) }, id: \.self) { item in
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
            Divider()  // style-exempt: menu
            ForEach(last, id: \.self) { item in
                Button(item.title, role: item == .remove ? .destructive : nil) { perform(item) }
            }
        }
    }
}
