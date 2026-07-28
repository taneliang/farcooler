import SwiftUI

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

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            header

            if isExpanded {
                ForEach(workspace.terminals) { t in
                    TerminalRow(
                        terminal: t,
                        isSelected: selection == .terminal(workspace: workspace.id, terminal: t.id),
                        onSelect: {
                            selection = .terminal(workspace: workspace.id, terminal: t.id)
                        },
                        onAction: { onTerminalAction(t, $0) }
                    )
                }

                Button(action: onNewTerminal) {
                    Label("New terminal", systemImage: "plus")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.vertical, 5)
                        .padding(.leading, 30)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.bottom, 4)
    }

    private var header: some View {
        HStack(spacing: 7) {
            Button(action: onToggle) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.tertiary)
                    .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    .frame(width: 12, height: 12)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            StateDot(state: workspace.state)

            VStack(alignment: .leading, spacing: 1) {
                Text(workspace.task)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                HStack(spacing: 4) {
                    Image(systemName: "arrow.triangle.branch").font(.system(size: 8))
                    Text(workspace.branch).lineLimit(1)
                }
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
            }

            Spacer(minLength: 4)

            if hovering {
                Button(action: onNewTerminal) {
                    Image(systemName: "plus").font(.system(size: 10, weight: .semibold))
                }
                .buttonStyle(.borderless)
                .help("New terminal")
            }

            Menu {
                Button("New terminal", action: onNewTerminal)
                Divider()
                Button("Archive", action: onArchive)
                Button("Remove worktree...", role: .destructive, action: onRemove)
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 10, weight: .semibold))
                    .frame(width: 16, height: 16)
                    .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .opacity(hovering ? 1 : 0.35)
        }
        .padding(.vertical, 6)
        .padding(.horizontal, 8)
        .background(
            RoundedRectangle(cornerRadius: 7)
                .fill(isSelected ? Color.accentColor.opacity(0.16) : (hovering ? Color.primary.opacity(0.05) : .clear))
        )
        .contentShape(Rectangle())
        .onTapGesture { selection = .workspace(workspace.id) }
        .onHover { hovering = $0 }
    }
}

/// One terminal row.
struct TerminalRow: View {
    let terminal: Terminal
    let isSelected: Bool
    let onSelect: () -> Void
    let onAction: (TerminalAction) -> Void

    @State private var hovering = false

    private var kind: StateKind { StateKind.parse(terminal.state) }

    var body: some View {
        HStack(spacing: 7) {
            StateDot(state: terminal.state)

            VStack(alignment: .leading, spacing: 1) {
                Text(terminal.title)
                    .font(.system(size: 12))
                    .lineLimit(1)
                HStack(spacing: 5) {
                    Text(terminal.preset)
                    if terminal.epoch > 0 {
                        Text("epoch \(terminal.epoch)")
                    }
                    if kind == .lost {
                        Text("lost").foregroundStyle(.red)
                    }
                }
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
            }

            Spacer(minLength: 4)

            if hovering || kind == .lost {
                Menu {
                    if kind == .lost {
                        Button("Restart") { onAction(.restart) }
                        Button("Dismiss loss") { onAction(.dismissLost) }
                    } else if kind == .exited {
                        Button("Restart") { onAction(.restart) }
                    } else {
                        Button("Restart") { onAction(.restart) }
                        Button("Stop", role: .destructive) { onAction(.stop) }
                    }
                } label: {
                    Image(systemName: "ellipsis")
                        .font(.system(size: 10, weight: .semibold))
                        .frame(width: 16, height: 16)
                        .contentShape(Rectangle())
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
            }
        }
        .padding(.vertical, 5)
        .padding(.leading, 27)
        .padding(.trailing, 8)
        .background(
            RoundedRectangle(cornerRadius: 7)
                .fill(isSelected ? Color.accentColor.opacity(0.16) : (hovering ? Color.primary.opacity(0.05) : .clear))
        )
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
    }
}

struct StateDot: View {
    let state: String

    private var color: Color {
        switch StateKind.parse(state) {
        case .running, .active: return .green
        case .starting: return .yellow
        case .exited, .ready: return .secondary
        case .lost, .error: return .red
        case .archived, .unknown: return .gray
        }
    }

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 7, height: 7)
            .help(state)
    }
}

/// Workspace detail: the facts, plus the actions.
struct WorkspaceDetail: View {
    let workspace: Workspace
    let onNewTerminal: () -> Void
    let onArchive: () -> Void
    let onRemove: () -> Void
    let onOpenTerminal: (Terminal) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 9) {
                        StateDot(state: workspace.state)
                        Text(workspace.task).font(.system(size: 26, weight: .semibold))
                    }
                    Text(workspace.branch)
                        .font(.system(size: 14, design: .monospaced))
                        .foregroundStyle(.secondary)
                }

                HStack(spacing: 9) {
                    Button {
                        onNewTerminal()
                    } label: {
                        Label("New terminal", systemImage: "plus")
                    }
                    .buttonStyle(.borderedProminent)

                    Button("Archive", action: onArchive)
                    Button("Remove worktree...", role: .destructive, action: onRemove)
                    Spacer()
                }

                facts

                if !workspace.terminals.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Terminals").font(.headline)
                        ForEach(workspace.terminals) { t in
                            Button {
                                onOpenTerminal(t)
                            } label: {
                                HStack(spacing: 9) {
                                    StateDot(state: t.state)
                                    Text(t.title)
                                    Text(t.preset).foregroundStyle(.secondary)
                                    Spacer()
                                    Text(t.state).foregroundStyle(.secondary).font(.caption)
                                    Image(systemName: "chevron.right")
                                        .font(.caption2)
                                        .foregroundStyle(.tertiary)
                                }
                                .padding(.vertical, 9)
                                .padding(.horizontal, 12)
                                .background(
                                    RoundedRectangle(cornerRadius: 8)
                                        .fill(Color.primary.opacity(0.04))
                                )
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
            .padding(32)
            .frame(maxWidth: 720, alignment: .leading)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var facts: some View {
        VStack(alignment: .leading, spacing: 10) {
            Fact(label: "State", value: workspace.state)
            Fact(label: "Terminals", value: "\(workspace.terminals.count)")
            Fact(label: "Worktree", value: workspace.worktree, monospaced: true)
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.primary.opacity(0.04)))
    }
}

struct Fact: View {
    let label: String
    let value: String
    var monospaced = false

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            Text(label)
                .font(.callout)
                .foregroundStyle(.secondary)
                .frame(width: 82, alignment: .leading)
            Text(value)
                .font(monospaced ? .system(.callout, design: .monospaced) : .callout)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
    }
}
