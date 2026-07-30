import SwiftUI

/// The sidebar's layout grid.
///
/// Written down because it was not before, and the result was every measurement
/// chosen locally: a workspace title at one indent, its terminals at another,
/// and the "new terminal" affordance at a third that sat to the LEFT of the
/// children it belonged to. Three columns pretending to be a hierarchy.
///
/// Everything below positions against these and nothing invents its own.
enum Grid {
    /// Every row's own horizontal padding, inside its highlight.
    ///
    /// Small on purpose. It used to be wide enough that a selected row's
    /// highlight began well to the LEFT of its parent's disclosure chevron, so
    /// children appeared to hang outside the workspace containing them. That is
    /// what made the list feel off balance: the indent said one thing and the
    /// highlight said the opposite.
    static let margin: CGFloat = 6

    /// The disclosure chevron's column — a gutter, like every outline view.
    static let chevron: CGFloat = 18

    /// The rail everything titled aligns to: workspace names, and the section
    /// header above them.
    static var rail: CGFloat { margin + chevron }

    /// One indent step. A child's marker sits here.
    static let indent: CGFloat = 16
    static var child: CGFloat { rail + indent }

    /// The marker column, reserved whether or not a row has anything to show,
    /// so titles align down the list.
    static let marker: CGFloat = 8
    static let gap: CGFloat = 8
    static var childText: CGFloat { child + marker + gap }

    /// Space between one workspace and the next.
    static let group: CGFloat = 12
}

/// One worktree, and its terminals when opened.
///
/// Collapsed to a single line by default. The previous version made a worktree
/// a two-line heading with its terminals always beneath it, which was built on
/// a wrong assumption about scale: there are a handful of projects, a handful
/// of terminals per worktree, and potentially hundreds of worktrees — some
/// dormant for weeks and resurrected later. The thing there are hundreds of has
/// to be the lightest row in the app, not the heaviest.
///
/// It expands on its own when something inside wants the user, because a
/// collapsed row that hides the agent asking a question defeats the point.
struct WorkspaceSection: View {
    let workspace: Workspace
    let isExpanded: Bool
    @Binding var selection: ContentView.Selection?
    let onToggle: () -> Void
    let onNewTerminal: () -> Void
    let onArchive: () -> Void
    let onRemove: () -> Void
    let onTerminalAction: (Terminal, TerminalAction) -> Void
    /// Where a terminal can be sent: each existing layout, plus a new one.
    ///
    /// The answer to "how do I tile these two together, and only these two". The
    /// keyboard can do it — ⌃B c then ⌃B a — but that is two steps in an order you
    /// have to know, and nothing on screen said groups existed. Moving a pane to a
    /// layout is one action and reads as what it is.
    var layouts: [PaneGroup] = []
    var onMoveToLayout: (Terminal, Int?) -> Void = { _, _ in }
    /// Dropping one terminal on another tiles the two together.
    var onDropTogether: (_ dragged: String, _ onto: Terminal) -> Void = { _, _ in }
    /// The terminals currently on screen together, if any.
    ///
    /// Passed in rather than looked up, because the sidebar's job is to show
    /// which of these rows you are looking at — the terminals that are tiled and
    /// the ones running behind them are the same list, and the difference is one
    /// mark, not a second section.
    var tiled: Set<String> = []

    @State private var hovering = false

    private var isSelected: Bool { selection == .workspace(workspace.id) }

    /// Open whether or not the user opened it.
    private var showsTerminals: Bool { isExpanded || !workspace.attention.isEmpty }

    private var ordered: [Terminal] {
        workspace.terminals.sorted { a, b in
            a.status.wantsAttention && !b.status.wantsAttention
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header

            if showsTerminals {
                ForEach(ordered) { t in
                    TerminalRow(
                        terminal: t,
                        isSelected: selection == .terminal(workspace: workspace.id, terminal: t.id),
                        onSelect: {
                            selection = .terminal(workspace: workspace.id, terminal: t.id)
                        },
                        isTiled: tiled.contains(t.id),
                        layouts: layouts,
                        onMoveToLayout: { onMoveToLayout(t, $0) },
                        onDropTogether: { onDropTogether($0, t) },
                        onAction: { onTerminalAction(t, $0) }
                    )
                }

                Button(action: onNewTerminal) {
                    HStack(spacing: Grid.gap) {
                        Image(systemName: "plus")
                            .font(.system(size: 9, weight: .semibold))
                            .frame(width: Grid.marker)
                        Text("New terminal").font(.system(size: 12))
                    }
                    .foregroundStyle(.tertiary)
                    .padding(.vertical, 4)
                    .padding(.leading, Grid.child)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .padding(.bottom, 4)
            }
        }
    }

    /// One line. Name, branch, and — only when closed — what is inside.
    private var header: some View {
        HStack(spacing: 0) {
            Button(action: onToggle) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .rotationEffect(.degrees(showsTerminals ? 90 : 0))
                    .frame(width: Grid.chevron, height: 16, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            Text(workspace.task)
                .font(.system(size: 13, weight: .medium))
                .lineLimit(1)

            Text(workspace.branch)
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
                .lineLimit(1)
                .padding(.leading, 7)
                .layoutPriority(-1)

            Spacer(minLength: 6)

            if !showsTerminals {
                // What a closed worktree still has to answer: is anything
                // happening in here?
                Text(workspace.summary)
                    .font(.system(size: 11))
                    .foregroundStyle(workspace.attention.isEmpty ? .tertiary : .secondary)
                    .lineLimit(1)
                    .layoutPriority(1)

                if !workspace.attention.isEmpty {
                    Circle().fill(Color.orange).frame(width: 6, height: 6)
                        .padding(.leading, 5)
                }
            }

            Menu {
                Button("New terminal", action: onNewTerminal)
                Divider()
                Button("Archive", action: onArchive)
                Button("Remove worktree…", role: .destructive, action: onRemove)
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 11, weight: .semibold))
                    .frame(width: 18, height: 16)
                    .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .opacity(hovering ? 1 : 0)
            .padding(.leading, 2)
        }
        .padding(.vertical, 4)
        .padding(.horizontal, Grid.margin)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(isSelected ? Color.primary.opacity(0.08) : (hovering ? Color.primary.opacity(0.04) : .clear))
        )
        .contentShape(Rectangle())
        .onTapGesture { selection = .workspace(workspace.id) }
        .onHover { hovering = $0 }
    }
}

/// A project heading.
///
/// Projects are chrome, not content: there are three or four and they change
/// rarely. A quiet label separating groups of worktrees is all the weight they
/// deserve.
struct ProjectHeader: View {
    let name: String
    let count: Int

    var body: some View {
        HStack(spacing: 6) {
            Text(name.uppercased())
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(.tertiary)
                .tracking(0.6)
            Text("\(count)")
                .font(.system(size: 10))
                .foregroundStyle(.quaternary)
                .monospacedDigit()
            Spacer()
        }
        .padding(.horizontal, Grid.margin + Grid.chevron)
        .padding(.top, 14)
        .padding(.bottom, 3)
    }
}

/// One terminal row.
///
/// A single line, deliberately. Both levels used to be two-line blocks, which
/// gave the list no rhythm: a workspace and the terminal under it were the same
/// shape and the same height, so nothing read as containing anything. A
/// two-line heading over single-line items is the difference between a tree and
/// a run of similar rectangles.
/// A layout's name, or its number when the name IS its number.
private func layoutLabel(_ group: PaneGroup, position: Int) -> String {
    group.name == "\(position)" ? "Move to layout \(position)" : "Move to \(group.name)"
}

struct TerminalRow: View {
    let terminal: Terminal
    let isSelected: Bool
    let onSelect: () -> Void
    /// On screen as part of the current layout.
    var isTiled: Bool = false
    var layouts: [PaneGroup] = []
    /// `nil` means a new layout of its own.
    var onMoveToLayout: (Int?) -> Void = { _ in }
    var onDropTogether: (String) -> Void = { _ in }
    let onAction: (TerminalAction) -> Void

    @State private var hovering = false
    @State private var targeted = false

    private var status: Status { terminal.status }

    /// Preset, and the status only when it is not the boring case.
    private var meta: String {
        guard status.wantsAttention || status == .working else { return terminal.preset }
        let label = terminal.statusDuration.map { "\(status.label) \($0)" } ?? status.label
        return "\(terminal.preset) · \(label)"
    }

    var body: some View {
        HStack(spacing: Grid.gap) {
            StatusGlyph(status: status, size: Grid.marker)

            Text(terminal.title)
                .font(.system(size: 13))
                .lineLimit(1)
                // The title yields before the status does. Which terminal it is
                // matters less than what it wants, and the sidebar is narrow.
                .layoutPriority(0)

            Text(meta)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .layoutPriority(1)

            Spacer(minLength: 0)

            // One small mark for "this one is on screen right now", so the
            // fourth agent running in the background is visibly the odd one out
            // rather than indistinguishable from the three you arranged.
            if isTiled {
                Image(systemName: "square.split.2x2")
                    .font(.system(size: 9))
                    .foregroundStyle(isSelected ? .primary : .tertiary)
            }

            if status == .lost {
                Button("Dismiss") { onAction(.dismissLost) }
                    .buttonStyle(.borderless)
                    .font(.system(size: 11))
            }
        }
        .padding(.vertical, 4)
        .padding(.leading, Grid.child)
        .padding(.trailing, Grid.margin)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(isSelected ? Color.primary.opacity(0.08) : (hovering ? Color.primary.opacity(0.04) : .clear))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .strokeBorder(Color.accentColor, lineWidth: targeted ? 2 : 0))
        )
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
        // Drag one terminal onto another and the two become a layout. Direct
        // manipulation for the thing the menu spells out in words: "tile these
        // two, and only these two" is awkward to describe and obvious to do.
        .draggable(terminal.id)
        .dropDestination(for: String.self) { items, _ in
            guard let dragged = items.first, dragged != terminal.id else { return false }
            onDropTogether(dragged)
            return true
        } isTargeted: { targeted = $0 }
        .contextMenu {
            Button("Tile on its own") { onMoveToLayout(nil) }
            if !layouts.isEmpty {
                Divider()
                ForEach(Array(layouts.enumerated()), id: \.element.id) { index, group in
                    Button(layoutLabel(group, position: index + 1)) {
                        onMoveToLayout(index + 1)
                    }
                    .disabled(group.terminals.contains(terminal.id))
                }
            }
            if isTiled {
                Divider()
                Button("Take off screen") { onMoveToLayout(-1) }
            }
        }
    }
}

/// A workspace's own status, for its heading.
///
/// Workspaces have three states worth showing and no agent of their own, so
/// this stays a small dot: it is context for the heading rather than something
/// to scan, and the terminals underneath carry the detail.
struct WorkspaceDot: View {
    let state: String

    private var colour: Color {
        switch StateKind.parse(state) {
        case .active: return .green
        case .error: return .red
        case .archived: return Color.secondary.opacity(0.4)
        default: return .secondary
        }
    }

    var body: some View {
        Circle().fill(colour).frame(width: 8, height: 8).help(state)
    }
}

/// Workspace detail.
///
/// Used to open with a State / Terminals / Worktree key-value table, which read
/// like a database inspector and gave the most screen to a filesystem path
/// nobody needs. The terminals ARE the workspace, so they lead; the path is a
/// footnote you can copy when you want it.
struct WorkspaceDetail: View {
    let workspace: Workspace
    /// Put every terminal here on screen together.
    var onTile: () -> Void = {}
    let onNewTerminal: () -> Void
    let onArchive: () -> Void
    let onRemove: () -> Void
    let onOpenTerminal: (Terminal) -> Void

    private var ordered: [Terminal] {
        workspace.terminals.sorted { a, b in
            a.status.wantsAttention && !b.status.wantsAttention
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                header

                if workspace.terminals.isEmpty {
                    empty
                } else {
                    VStack(spacing: 8) {
                        ForEach(ordered) { t in
                            terminalCard(t)
                        }
                    }
                }

                footnote
            }
            .padding(32)
            .frame(maxWidth: 720, alignment: .leading)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // Same title as the tiled and solo views, so switching between them does
        // not change what the window claims you are looking at.
        .navigationTitle(workspace.windowTitle)
        .navigationSubtitle(workspace.windowSubtitle)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 10) {
                    WorkspaceDot(state: workspace.state)
                    Text(workspace.task).font(.system(size: 24, weight: .semibold))
                }
                Text(workspace.branch)
                    .font(.system(size: 13, design: .monospaced))
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 8) {
                Button(action: onNewTerminal) {
                    Label("New terminal", systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("t", modifiers: .command)

                // The way in that is not a keystroke. Tiling is keyboard-first
                // and the prefix is the fast path, but a feature reachable only
                // by a binding you have to be told about does not exist for
                // anyone who has not been told.
                if workspace.terminals.count > 1 {
                    Button(action: onTile) {
                        Label("Tile all", systemImage: "square.split.2x2")
                    }
                }

                Spacer()

                // Destructive actions live in a menu rather than sitting as
                // permanent buttons next to the one you press constantly.
                Menu {
                    Button("Archive", action: onArchive)
                    Button("Remove worktree…", role: .destructive, action: onRemove)
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
            }
        }
    }

    private var empty: some View {
        VStack(spacing: 8) {
            Image(systemName: "terminal")
                .font(.system(size: 28))
                .foregroundStyle(.tertiary)
            Text("No terminals").font(.callout.weight(.medium))
            Text("A terminal runs one agent, or one shell, inside this worktree.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 40)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.primary.opacity(0.03)))
    }

    private func terminalCard(_ t: Terminal) -> some View {
        Button {
            onOpenTerminal(t)
        } label: {
            HStack(spacing: 12) {
                StatusGlyph(status: t.status, size: 10)

                VStack(alignment: .leading, spacing: 2) {
                    Text(t.title).font(.system(size: 14, weight: .medium))
                    Text(t.preset).font(.system(size: 12)).foregroundStyle(.secondary)
                }

                Spacer()

                Text(t.statusDuration.map { "\(t.status.label) \($0)" } ?? t.status.label)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)

                Image(systemName: "chevron.right")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.vertical, 12)
            .padding(.horizontal, 14)
            .background(
                RoundedRectangle(cornerRadius: 10)
                    .fill(Color.primary.opacity(0.04))
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    /// The path, demoted to where it belongs.
    private var footnote: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "folder")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
            Text(workspace.worktree)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.tertiary)
                .textSelection(.enabled)
                .lineLimit(2)
                .truncationMode(.middle)
        }
    }
}
