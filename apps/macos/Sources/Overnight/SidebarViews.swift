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

/// One workspace and its terminals in the sidebar.
struct WorkspaceSection: View {
    let workspace: Workspace
    let isExpanded: Bool
    @Binding var selection: ContentView.Selection?
    let onToggle: () -> Void
    let onNewTerminal: () -> Void
    let onArchive: () -> Void
    let onRemove: () -> Void
    let onTerminalAction: (Terminal, TerminalAction) -> Void

    @State private var hovering = false

    private var isSelected: Bool {
        selection == .workspace(workspace.id)
    }

    /// Terminals wanting attention first.
    ///
    /// A fleet is read top-down and scanning past four idle shells to find the
    /// one asking a question is the work this screen exists to remove.
    private var ordered: [Terminal] {
        workspace.terminals.sorted { a, b in
            a.status.wantsAttention && !b.status.wantsAttention
        }
    }

    private var attentionCount: Int {
        workspace.terminals.filter(\.status.wantsAttention).count
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header

            if isExpanded {
                ForEach(ordered) { t in
                    TerminalRow(
                        terminal: t,
                        isSelected: selection == .terminal(workspace: workspace.id, terminal: t.id),
                        onSelect: {
                            selection = .terminal(workspace: workspace.id, terminal: t.id)
                        },
                        onAction: { onTerminalAction(t, $0) }
                    )
                }

                // Deliberately not styled as a row. It used to look like a
                // terminal that happened to be called "New terminal", which is
                // a thing you read before realising it is a button.
                // Aligned with the terminals' text, not with their dots and not
                // with their parent. It used to sit left of the rows it adds to.
                Button(action: onNewTerminal) {
                    HStack(spacing: Grid.gap) {
                        Image(systemName: "plus")
                            .font(.system(size: 9, weight: .semibold))
                            .frame(width: Grid.marker)
                        Text("New terminal").font(.system(size: 12))
                    }
                    .foregroundStyle(.tertiary)
                    .padding(.vertical, 4)
                    // Its icon sits in the status-dot column and its text in
                    // the title column, so it reads as another row in the same
                    // list rather than something floating beside it.
                    .padding(.leading, Grid.child)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.bottom, Grid.group)
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 0) {
            Button(action: onToggle) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    // Sized to the grid, so the title starts at a known column
                    // rather than wherever the glyph happened to end.
                    .frame(width: Grid.chevron, height: 17, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            // A tight two-line block, not two competing lines. The branch was
            // set in monospace at nearly the title's size, which made every
            // workspace read as two headings stacked on each other.
            VStack(alignment: .leading, spacing: 0) {
                Text(workspace.task)
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(1)
                Text(workspace.branch)
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
            .padding(.leading, 0)

            Spacer(minLength: 8)
                .frame(height: 17)

            // Survives collapsing. Otherwise the agent asking for permission is
            // hidden behind a disclosure triangle, in the app you opened to
            // find out what needs you.
            if !isExpanded {
                AttentionBadge(count: attentionCount)
            }

            Menu {
                Button("New terminal", action: onNewTerminal)
                Divider()
                Button("Archive", action: onArchive)
                Button("Remove worktree…", role: .destructive, action: onRemove)
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 11, weight: .semibold))
                    .frame(width: 18, height: 18)
                    .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            // Hidden until wanted. A row with a permanent control on it reads
            // as busier than it is.
            .opacity(hovering ? 1 : 0)
        }
        .padding(.vertical, 5)
        .padding(.horizontal, Grid.margin)
        .background(
            RoundedRectangle(cornerRadius: 6)
                // Neutral, not accent-tinted. Selection says where you are;
                // it must not compete with the one colour that says what needs
                // you.
                .fill(isSelected ? Color.primary.opacity(0.08) : (hovering ? Color.primary.opacity(0.04) : .clear))
        )
        .contentShape(Rectangle())
        .onTapGesture { selection = .workspace(workspace.id) }
        .onHover { hovering = $0 }
    }
}

/// One terminal row.
///
/// A single line, deliberately. Both levels used to be two-line blocks, which
/// gave the list no rhythm: a workspace and the terminal under it were the same
/// shape and the same height, so nothing read as containing anything. A
/// two-line heading over single-line items is the difference between a tree and
/// a run of similar rectangles.
struct TerminalRow: View {
    let terminal: Terminal
    let isSelected: Bool
    let onSelect: () -> Void
    let onAction: (TerminalAction) -> Void

    @State private var hovering = false

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
        )
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
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
