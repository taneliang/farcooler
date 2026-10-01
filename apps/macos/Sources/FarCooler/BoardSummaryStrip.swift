import AgentKit
import SwiftUI

/// "Since you were last here": a compact, collapsible strip at the top of the
/// board listing what changed over a period, each line opening its task.
///
/// The board's header hosts it. What goes in the strip is `BoardSummary`'s;
/// this view only chooses the period, draws the lines and reads the records of
/// the tasks that moved.
struct BoardSummaryStrip: View {
    @ObservedObject var store: TaskBoardStore
    let defaults: UserDefaults

    @State private var period: BoardSummary.Period
    @State private var collapsed: Bool

    init(store: TaskBoardStore, defaults: UserDefaults = .standard) {
        self.store = store
        self.defaults = defaults
        _period = State(initialValue: Self.readPeriod(store, defaults))
        _collapsed = State(initialValue: defaults.bool(forKey: Self.collapsedKey(store)))
    }

    private static func collapsedKey(_ store: TaskBoardStore) -> String {
        "board.summary.collapsed.\(store.hostKey).\(store.workspace.id)"
    }

    private static func periodKey(_ store: TaskBoardStore) -> String {
        "board.summary.period.\(store.hostKey).\(store.workspace.id)"
    }

    private static func readPeriod(_ store: TaskBoardStore, _ defaults: UserDefaults) -> BoardSummary.Period {
        defaults.string(forKey: periodKey(store)).flatMap(BoardSummary.Period.init(rawValue:))
            ?? .sinceLastVisit
    }

    var body: some View {
        BoardTick { now in
            let since = BoardSummary.start(of: period, lastVisit: store.visitBaseline, now: now)
            let summary = BoardSummary.make(
                rows: store.board.rows, notes: store.summaryNotes, since: since)
            VStack(alignment: .leading, spacing: 6) {
                header
                if !collapsed {
                    if summary.isEmpty {
                        Text(BoardSummary.nothingNew)
                            .font(.system(size: WorkspaceStyle.PaneText.body))
                            .foregroundStyle(.secondary)
                            .accessibilityIdentifier("board-summary-empty")
                    } else {
                        group("Finished", summary.finished)
                        group("Needs You or Review", summary.moved)
                        group("New", summary.created)
                        group("Decisions and Findings", summary.notes)
                    }
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(WorkspaceStyle.document)
            .task(id: Self.notesKey(since: since, generation: store.generation, count: store.board.rows.count, collapsed: collapsed)) {
                guard !collapsed else { return }
                await store.readSummaryNotes(since: since)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-summary")
    }

    /// What a notes read is keyed on. `collapsed` is in it so a strip expanded
    /// after launch reads its decisions and findings then, not at the next minute.
    struct NotesKey: Hashable {
        var since: Double
        var generation: Int
        var count: Int
        var collapsed: Bool
    }

    static func notesKey(since: Date, generation: Int, count: Int, collapsed: Bool) -> NotesKey {
        NotesKey(
            since: (since.timeIntervalSince1970 / 60).rounded(.down), generation: generation,
            count: count, collapsed: collapsed)
    }

    private var header: some View {
        HStack(spacing: 6) {
            Button {
                collapsed.toggle()
                defaults.set(collapsed, forKey: Self.collapsedKey(store))
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .rotationEffect(.degrees(collapsed ? 0 : 90))
                        .foregroundStyle(.secondary)
                    Text("Since you were last here")
                        .font(.system(size: WorkspaceStyle.PaneText.body, weight: .semibold))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(collapsed ? "Collapsed" : "Expanded")
            .accessibilityIdentifier("board-summary-toggle")
            Spacer(minLength: 0)
            if !collapsed {
                Picker("Period", selection: $period) {
                    ForEach(BoardSummary.Period.allCases) { Text($0.title).tag($0) }
                }
                .labelsHidden()
                .pickerStyle(.menu)
                .fixedSize()
                .accessibilityIdentifier("board-summary-period")
                .onChange(of: period) { _, now in
                    defaults.set(now.rawValue, forKey: Self.periodKey(store))
                }
            }
        }
    }

    @ViewBuilder private func group(_ title: String, _ items: [BoardSummary.Item]) -> some View {
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: 2) {
                Text("\(title) (\(items.count))")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                    .foregroundStyle(.secondary)
                ForEach(BoardSummary.capped(items).shown) { item in
                    Button {
                        if let row = store.board.rows.first(where: { $0.id == item.taskID }) {
                            store.choose(row)
                        }
                    } label: {
                        HStack(spacing: 6) {
                            Text(item.key)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                                .foregroundStyle(.secondary)
                            Text(item.title).lineLimit(1)
                            if let detail = item.detail {
                                Text(detail)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                            }
                            Spacer(minLength: 0)
                        }
                        .font(.system(size: WorkspaceStyle.PaneText.body))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("board-summary-item-\(item.id)")
                }
                if BoardSummary.capped(items).more > 0 {
                    Text("and \(BoardSummary.capped(items).more) more")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}
