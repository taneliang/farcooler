import SwiftUI

// One repository's board, on a phone: read it, and go to the agent on a card.
//
// Opened from a Board row at the top of a runner's section in the shell
// overview (`ShellOverview.boardRow`), and presented as a sheet by
// `ShellScreen` rather than pushed into the overview's own stack — the
// overview is unmounted the moment the grid stops showing, so a push into it
// is a push whose stack can vanish under it (see `authorizingDevice` there).
//
// A list grouped by status rather than the Mac's seven columns: seven columns
// side by side do not fit a phone, and the order is the Mac's — Needs Decision
// first, because it is the one status waiting on the person reading. Every
// sentence and every rule on a card is AgentKit's (`TaskBoardModel`,
// `TaskBoardAgents`, `RunnerBoards`), so this phone and the Mac say the same
// thing about the same card. This file draws.
//
// Read-only in this phase. Moving a card and answering a question are writes
// with their own rules, and they come with the screen that makes them.

/// A pane a card can go to, as the board draws it.
struct BoardAgent: Identifiable, Equatable {
    /// The terminal's id, which is what `ShellScreen` resolves to a tab.
    let id: String
    /// The menu item's words: `claude in fix-reconnect`, told apart from a
    /// twin by its short id. See `TaskAgentLink.menuTitles`.
    let title: String
    /// The pane's own mark, so an agent waiting on a question is amber here
    /// too — the card says which agent needs you before you go.
    let mark: GlanceMark
}

/// What a board sheet is about, and where it came from.
struct BoardSheet: Identifiable {
    let runner: String
    let repository: String
    let name: String
    var id: String { "\(runner)/\(repository)" }
}

struct TaskBoardView: View {
    let name: String
    /// The board as last read, or nil when it has never been read.
    let board: TaskBoardModel?
    /// Whether the last read failed. With a board in hand, that board is still
    /// drawn and a line says it may be behind.
    let unread: Bool
    /// Whether this runner can be believed about its panes right now — see
    /// `TaskAgentLink.speaksOfAgents`. False draws no agent control at all.
    let speaksOfAgents: Bool
    /// The panes working a card, in fleet order.
    let agents: (TaskRow) -> [BoardAgent]
    let onJump: (BoardAgent) -> Void
    let onRefresh: () async -> Void
    let onDone: () -> Void

    var body: some View {
        NavigationStack {
            content
                .navigationTitle(name)
                .navigationBarTitleDisplayMode(.inline)
                // The one count worth putting at the top, in the Mac's words.
                .navigationSubtitle(board?.waitingSentence ?? "")
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done", action: onDone)
                            .accessibilityIdentifier("board-done")
                    }
                }
                .refreshable { await onRefresh() }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board")
    }

    @ViewBuilder
    private var content: some View {
        if let board {
            if board.rows.isEmpty && board.unreadable.isEmpty {
                ContentUnavailableView {
                    Label("No Tasks", systemImage: "checklist")
                } description: {
                    Text("Nothing is on this board yet.")
                }
            } else {
                list(board)
            }
        } else if unread {
            ContentUnavailableView {
                Label("Couldn’t Read This Board", systemImage: "exclamationmark.triangle")
            } description: {
                Text("Far Cooler couldn’t read this board. Pull down to try again.")
            }
        } else {
            ProgressView()
        }
    }

    private func list(_ board: TaskBoardModel) -> some View {
        List {
            if unread {
                Section {
                    Label(
                        "Couldn’t read this board just now. This is how it was last read.",
                        systemImage: "exclamationmark.triangle"
                    )
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                }
            }
            ForEach(board.listed) { column in
                Section {
                    ForEach(column.rows) { row in
                        let live = speaksOfAgents ? agents(row) : []
                        TaskBoardCardRow(
                            row: row,
                            live: live,
                            presence: row.agentPresence(
                                livePanes: live.count, runnerRecordsTasks: speaksOfAgents),
                            onJump: onJump)
                    }
                } header: {
                    HStack(spacing: PaneMetrics.tight) {
                        Text(column.title)
                        Text("\(column.rows.count)")
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(.tertiary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("board-section-\(column.id)")
                }
            }
            // Rows this build has no status for: carried and shown under a
            // heading that says it is this app that is behind, never dropped
            // and never filed under a status they are not in.
            if !board.unreadable.isEmpty {
                Section("Not On This Version") {
                    ForEach(board.unreadable) { row in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(row.key)
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                            Text(row.title)
                            Text(row.status)
                                .font(.system(.caption2, design: .monospaced))
                                .foregroundStyle(.tertiary)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
    }
}

/// One card: its key, its title, what it asks of you, how long it has sat, how
/// much of its acceptance holds, and the way to its agent.
private struct TaskBoardCardRow: View {
    let row: TaskRow
    let live: [BoardAgent]
    let presence: TaskAgentPresence
    let onJump: (BoardAgent) -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(alignment: .top, spacing: PaneMetrics.step) {
            VStack(alignment: .leading, spacing: PaneMetrics.tight) {
                HStack(spacing: PaneMetrics.tight) {
                    // Mono, because a key is typed into a terminal, and the
                    // brief's rule is that mono means data.
                    Text(row.key)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(.secondary)
                    if row.staleness == .stale {
                        Image(systemName: "clock")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                            .accessibilityHidden(true)
                    }
                }
                Text(row.title)
                    .font(.subheadline)
                    .lineLimit(3)
                if let ask = row.callToAction {
                    // Amber, because Needs Decision is the one status waiting
                    // on the person reading — which is what amber means here.
                    Text(ask)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(GlancePalette.amber(scheme))
                }
                if let note = row.stalenessNote(at: Date()) {
                    Text(note)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if let progress = row.acceptanceProgress {
                    AcceptanceLine(progress: progress)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // One element for the card's words, and the control beside it its
            // own: an identifier on the `HStack` would be pushed down onto the
            // Agent button too and rename it.
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("board-card-\(row.key)")

            AgentControl(key: row.key, live: live, presence: presence, onJump: onJump)
        }
        .padding(.vertical, 2)
    }
}

/// `2 of 5`, or `All 5 met` in the accent color once every line holds.
private struct AcceptanceLine: View {
    let progress: TaskAcceptanceProgress

    var body: some View {
        // An `HStack` and not a `Label`: inside a `List` a label's icon gets
        // the row's icon column, which set the words a thumb's width away from
        // the glyph they belong to.
        HStack(spacing: 3) {
            Image(systemName: progress.isComplete ? "checkmark.circle.fill" : "checkmark.circle")
            Text(progress.sentence).monospacedDigit()
        }
        .accessibilityElement(children: .ignore)
        .font(.caption.weight(progress.isComplete ? .medium : .regular))
        .foregroundStyle(progress.isComplete ? Color.accentColor : Color.secondary)
        .accessibilityLabel("Acceptance: \(progress.sentence)")
    }
}

/// The way from a card to the agent working it: a button for one, a menu for
/// several, a quiet "No Agent" for a task in progress with nobody on it, and
/// nothing on a runner that cannot say.
private struct AgentControl: View {
    let key: String
    let live: [BoardAgent]
    let presence: TaskAgentPresence
    let onJump: (BoardAgent) -> Void

    var body: some View {
        switch presence {
        case .unsaid:
            EmptyView()
        case .noAgent:
            Text(presence.title ?? "")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .accessibilityIdentifier("board-no-agent-\(key)")
        case .agents:
            if live.count == 1, let only = live.first {
                Button {
                    onJump(only)
                } label: {
                    pill(mark: only.mark, trailing: "arrow.forward")
                }
                .buttonStyle(.bordered)
                .buttonBorderShape(.capsule)
                .controlSize(.small)
                .accessibilityLabel("Go to Agent")
                .accessibilityHint(only.title)
                .accessibilityIdentifier("board-agent-\(key)")
            } else {
                Menu {
                    ForEach(live) { agent in
                        Button {
                            onJump(agent)
                        } label: {
                            Text(agent.title)
                        }
                    }
                } label: {
                    pill(mark: live.first?.mark, trailing: "chevron.down")
                }
                .buttonStyle(.bordered)
                .buttonBorderShape(.capsule)
                .controlSize(.small)
                .accessibilityLabel(presence.title ?? "Agents")
                .accessibilityIdentifier("board-agent-\(key)")
            }
        }
    }

    private func pill(mark: GlanceMark?, trailing: String) -> some View {
        HStack(spacing: PaneMetrics.tight) {
            if let mark { ShellMarkView(mark: mark, size: 7) }
            Text(presence.title ?? "")
            Image(systemName: trailing)
                .font(.caption2.weight(.bold))
        }
        .font(.caption.weight(.medium))
    }
}
