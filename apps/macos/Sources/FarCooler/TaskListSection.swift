import AgentKit
import SwiftUI

// Moved out of TaskBoard.swift (ov-177), whole, for the file-size budget.

/// One status in the list form: its header, and its rows when open.
///
/// An empty status is a header reading "Backlog 0" that can't open, so the
/// list says what isn't there as well as what is. Done and Canceled are cut
/// by `BoardDone`'s rule with a row to the History page under them; a long
/// section shows ten, and "Show N More" (ov-103).
struct TaskListSection: View {
    let section: TaskBoardColumn
    let expanded: Bool
    let onToggle: () -> Void
    @ObservedObject var store: TaskBoardStore
    let agents: BoardAgents
    let onGoTo: (BoardPane) -> Void
    /// The task open beside the board, which the section keeps though
    /// opening it read it; the row lit, the same task unless it's lit on its
    /// Unread line (ov-177); and whether the list has the keyboard, which
    /// draws it in the accent rather than gray.
    let selected: String?
    let lit: String?
    let keyed: Bool
    /// Each task's worktree, and its menu.
    let worktrees: BoardWorktrees
    /// Showing all of a long section.
    let showingMore: Bool
    let onShowMore: () -> Void
    /// The navigator's filter is narrowing it: every match shows.
    let filtering: Bool
    /// The History row: its status's page in the main area.
    let onHistory: (TaskStatus) -> Void
    let onChoose: (TaskRow) -> Void
    let ask: AskOrchestrator.Action
    /// Where a row moving between statuses is matched, so it moves rather
    /// than leaving one section and appearing in another.
    let rows: Namespace.ID

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.boardMotionSlowdown) private var slowdown

    /// Needs Decision is the one status waiting on the person reading, and
    /// the only one drawn in the accent color.
    private var leads: Bool { section.status == .needsDecision }

    var body: some View {
        // On the board's tick, so Done's "today" turns over at midnight on a
        // board nothing else redraws (ov-104 review).
        BoardTick { now in
            content(section.cut(
                reads: store.reads, keeping: selected, showingAll: showingMore, filtering: filtering, now: now))
        }
    }

    private func content(_ cut: TaskBoardColumn.Cut) -> some View {
        // Its chevron at column A, its title at B, its count trailing
        // (ov-83), through the board's one collapsible section (ov-92).
        CollapsibleSection(
            section.title, id: "status.\(section.status.rawValue)",
            tone: section.count == 0 ? .quiet : leads ? .accent : .primary,
            isExpanded: Binding(get: { expanded }, set: { open in if open != expanded { onToggle() } }),
            canExpand: BoardForm.canExpand(section), count: section.count
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(cut.rows) { row in
                    TaskListRow(
                        row: row, prominent: leads, store: store,
                        live: agents.live(for: row), presence: agents.presence(for: row),
                        onGoTo: onGoTo, orchestrator: agents.orchestrator(for: row),
                        speaksOfAgents: agents.runnerRecordsTasks, selected: row.id == lit, keyed: keyed,
                        worktree: worktrees.byTask[row.id],
                        worktreeMenu: worktrees.byTask[row.id].map(worktrees.menu) ?? [],
                        performOnWorktree: { item in
                            if let worktree = worktrees.byTask[row.id] { worktrees.perform(item, worktree) }
                        },
                        onChoose: { onChoose(row) }, ask: ask)
                    .matchedGeometryEffect(id: row.id, in: rows, isSource: true)
                    .transition(BoardMotion.rowTransition(reduceMotion: reduceMotion, slowedBy: slowdown))
                    .id(row.id)
                }
                if cut.hidden > 0 {
                    SectionFootButton(
                        title: BoardSectionCut.showMoreTitle(cut.hidden),
                        id: "board-show-more-\(section.status.rawValue)", action: onShowMore)
                } else if showingMore, !filtering, !section.status.isFinished,
                    section.rows.count > BoardSectionCut.limit
                {
                    SectionFootButton(
                        title: "Show Fewer", id: "board-show-fewer-\(section.status.rawValue)", action: onShowMore)
                }
                if let total = cut.history, !filtering {
                    HistoryRow(status: section.status, total: total) { onHistory(section.status) }
                }
            }
            .animation(BoardMotion.list(reduceMotion: reduceMotion, slowedBy: slowdown), value: cut.rows.map(\.id))
        }
    }
}

/// "Show 4 More" at the foot of a long section, at column B.
struct SectionFootButton: View {
    let title: String
    let id: String
    let action: () -> Void

    var body: some View {
        Button(title, action: action)
            .buttonStyle(.plain)
            .font(.system(size: WorkspaceStyle.PaneText.secondary))
            .foregroundStyle(.secondary)
            .frame(minHeight: 2 * ColumnGrid.rhythm)
            .padding(.leading, NavigatorGrid.textInset)
            .accessibilityIdentifier(id)
    }
}

/// "All Done  94 ›": every task in the status, on the History page (ov-103).
/// Its count trailing as a header's is, never in parentheses.
struct HistoryRow: View {
    let status: TaskStatus
    let total: Int
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Text(BoardDone.historyTitle(status))
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
                Spacer(minLength: SidebarGrid.gap)
                SectionCount(count: total)
                // Forward, a way to a page: not a disclosure's chevron.
                Image(systemName: "chevron.forward")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.leading, NavigatorGrid.textInset)
            .padding(.trailing, ColumnGrid.rhythm)
            .frame(minHeight: ColumnGrid.rowHeight)
            .background(RoundedRectangle(cornerRadius: 8).fill(Color.primary.opacity(hovering ? 0.06 : 0)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .accessibilityLabel("\(BoardDone.historyTitle(status)), \(total)")
        .accessibilityIdentifier("board-history-\(status.rawValue)")
    }
}
