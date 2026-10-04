import AgentKit
import SwiftUI

/// A repository's own terminals (ov-178, from ov-190): the ones started by
/// hand in its main checkout, for what belongs to the project and to no task,
/// a proxy, a log tail, a long-running script. The owner: "a way to run
/// terminals on the project level".
///
/// They have a section of their own in every one of the repository's
/// workspaces, beside the task list: never in it, never in Unread, and never
/// started for you. A worktree's own terminals stay its own, reached from its
/// row. Worked out here as values, so which terminals count, and when the
/// section is drawn, are the ones `ProjectTerminalsTests` pins.
struct ProjectTerminals {
    /// The repository's main checkout, where they run.
    var checkout: Worktree?
    /// Its terminals that are the project's (`terminals(in:fleet:)`).
    var terminals: [Terminal] = []
    /// The one open in the main area, by id.
    var selected: String?
    var onOpen: (Terminal) -> Void = { _ in }
    /// Restart or Dismiss, from a lost terminal's context menu: ov-191's
    /// answers, as the worktree page's cards offer them.
    var onAction: (TerminalAction, Terminal) -> Void = { _, _ in }
    /// New Terminal, the section's trailing row. Nil where the runner can't
    /// take one.
    var onNew: (() -> Void)?
    /// Whether the runner can name a terminal (`terminal_names`): Rename… is
    /// offered only where it can.
    var canRename = false
    /// The runner is this Mac, so a port opens in its browser.
    var onThisMac = false

    static var none: ProjectTerminals { ProjectTerminals() }

    /// Whether the navigator draws the section: with terminals to list, or
    /// one to start.
    var isShown: Bool { !terminals.isEmpty || onNew != nil }

    /// The main checkout of `workspace`'s repository on `host`, which every
    /// one of its workspaces shares. A repository's implicit workspace is
    /// keyed by the repository itself.
    static func checkout(for workspace: WorkspaceSummary, host: String, in fleet: Fleet) -> Worktree? {
        let repository = workspace.repository ?? workspace.id
        return fleet.worktrees.first {
            $0.isMainCheckout && ($0.host ?? "") == host && $0.repositoryID == repository
        }
    }

    /// The checkout's terminals that are the project's: not an orchestrator,
    /// which is its workspace's conversation (seated or stopped); not a
    /// task's agent, which its task's row has; and not a changes pane, which
    /// belongs to the diff it draws. In the runner's order.
    static func terminals(in checkout: Worktree, fleet: Fleet) -> [Terminal] {
        WorkspaceScreen.ownTerminals(of: checkout, fleet: fleet).terminals.filter {
            !$0.isOrchestrator && $0.taskId == nil && !$0.isChangesPane
        }
    }

    /// The ids of `workspace`'s repository's project terminals: what lights a
    /// Terminals row rather than the checkout's (ov-234).
    static func ids(for workspace: WorkspaceSummary, host: String, in fleet: Fleet) -> Set<String> {
        guard let checkout = checkout(for: workspace, host: host, in: fleet) else { return [] }
        return Set(terminals(in: checkout, fleet: fleet).map(\.id))
    }

    /// The name of `terminal` when it's one of `worktree`'s project terminals
    /// (`terminals(in:fleet:)`) and `worktree` is a main checkout, else nil.
    /// What the breadcrumb says for one open (ov-234).
    static func name(of terminal: String, in worktree: Worktree?, fleet: Fleet) -> String? {
        guard let worktree, worktree.isMainCheckout else { return nil }
        return terminals(in: worktree, fleet: fleet).first { $0.id == terminal }?.label
    }

    /// What the section draws while the navigator's filter holds `filter`:
    /// the terminals whose name or command matches, and no New Terminal row,
    /// which would read as a hit. Itself with no filter.
    func narrowed(by filter: String) -> ProjectTerminals {
        guard !BoardFilter.isEmpty(filter) else { return self }
        var narrowed = self
        narrowed.terminals = terminals.filter { BoardFilter.matches(key: $0.label, title: $0.preset, filter) }
        narrowed.onNew = nil
        return narrowed
    }
}

/// The navigator's Terminals section (ov-178): the repository's own
/// terminals, each opened in the main area on a click, and New Terminal
/// trailing. Its header is the navigator's (`CollapsibleSection`), between
/// Tasks and Worktrees.
///
/// On the navigator's grid: a terminal's glyph in the glyph column, its name on
/// the text column.
struct ProjectTerminalsSection: View {
    let terminals: ProjectTerminals
    /// The list has the keyboard: a selected row reads in the accent.
    let keyed: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
            ForEach(terminals.terminals) { terminal in
                ProjectTerminalRow(
                    terminal: terminal, selected: terminal.id == terminals.selected, keyed: keyed,
                    canRename: terminals.canRename, onThisMac: terminals.onThisMac,
                    onOpen: { terminals.onOpen(terminal) }, onAction: { terminals.onAction($0, terminal) })
                .id(NavigatorItem.terminal(terminal.id))
            }
            if let onNew = terminals.onNew {
                Button(action: onNew) {
                    HStack(spacing: 0) {
                        Image(systemName: "plus")
                            .font(.system(size: 10, weight: .medium))
                            .gridMark("projectTerminalNew", .icon)
                            .glyphColumn()
                        Text("New Terminal")
                            .font(.system(size: WorkspaceStyle.PaneText.body))
                            .gridMark("projectTerminalNew", .text)
                        Spacer(minLength: 0)
                    }
                    .foregroundStyle(.secondary)
                    .frame(minHeight: ColumnGrid.rowHeight)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Start a terminal in the repository’s main checkout")
                .accessibilityLabel("New Terminal")
                .accessibilityIdentifier("navigator-new-terminal")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("navigator-terminals")
    }
}

/// One of the project's terminals: its glyph and name, and its status where
/// it has one worth a mark (`StatusGlyph`), a lost one's included.
private struct ProjectTerminalRow: View {
    let terminal: Terminal
    let selected: Bool
    let keyed: Bool
    var canRename = false
    var onThisMac = false
    let onOpen: () -> Void
    let onAction: (TerminalAction) -> Void

    var body: some View {
        // A button, so it's reached by the keyboard as well as the mouse.
        Button(action: onOpen) {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                Image(systemName: "terminal")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.secondary)
                    .gridMark("projectTerminal", .icon)
                    .glyphColumn()
                Text(terminal.label)
                    .font(.system(size: WorkspaceStyle.PaneText.body))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .gridMark("projectTerminal", .text)
                Spacer(minLength: SidebarGrid.gap)
                // What it serves, as the kernel says: `:5173`.
                if let port = terminal.portLabel {
                    Text(port)
                        .font(.system(size: WorkspaceStyle.PaneText.body))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .fixedSize()
                }
                StatusGlyph(status: terminal.status)
                    .help(terminal.status.label)
            }
            .navigatorRow(selected: selected, keyed: keyed, minHeight: ColumnGrid.rowHeight, leading: 0, box: "projectTerminal")
        }
        .buttonStyle(.plain)
        .contextMenu { menu }
        .accessibilityLabel(terminal.label)
        .accessibilityValue(terminal.status.label)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier("navigator-terminal-\(terminal.id)")
    }

    /// Open, with its port Open in Browser on this Mac, Rename… and Close, and
    /// for a terminal with no running pane the lost page's own answers
    /// (ov-191), as `WorktreeDetail`'s cards have them.
    @ViewBuilder
    private var menu: some View {
        Button("Open", action: onOpen)
        if onThisMac, terminal.portLabel != nil {
            Button("Open in Browser") { onAction(.openInBrowser) }
        }
        if canRename {
            Button("Rename…") { onAction(.rename) }
        }
        if LostPane.Kind(state: terminal.state) == nil {
            Button("Close") { onAction(.close) }
        }
        if let kind = LostPane.Kind(state: terminal.state) {
            Divider()  // style-exempt: menu section break
            ForEach(LostPane.actions(for: kind), id: \.title) { action in
                Button(action.title) { onAction(TerminalAction(action)) }
            }
        }
    }
}
