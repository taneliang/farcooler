import SwiftUI

/// A finished status's History page on a phone (ov-103), pushed from the
/// board's "All Done" row: every task in Done or Canceled, grouped Today,
/// Yesterday, This Week and Earlier, searchable by key and title, and
/// narrowed by the area chips its titles name. The Mac's page searches note
/// text too, through the runner's record; a phone can't ask for that yet.
struct BoardHistoryScreen: View {
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace
    let status: TaskStatus
    /// The plan footer's list this is (ov-874): "waiting for a builder" or
    /// "not started", whose tickets are the plan's, not one board column.
    /// Nil for a finished status's History.
    var footer: PlanColumn.Count.Kind?
    /// A pipeline stage's tickets nobody is on (ov-874): what the stage's
    /// "N more" line opens. Nil for the other lists.
    var stage: PlanColumn.Stage?

    @State private var query = ""
    @State private var area: String?
    @Environment(\.phoneNavigator) private var navigator

    /// The plan, as the plan column reads it; its lists are this screen's.
    private var column: PlanColumn? {
        guard case .loaded(let plan)? = connection.plans.state(place.workspace) else { return nil }
        let board = connection.boards[place.workspace]?.rows ?? []
        return PlanColumn(
            plan: plan, statuses: Dictionary(board.map { ($0.id, $0.status) }, uniquingKeysWith: { first, _ in first }),
            titles: Dictionary(board.map { ($0.id, $0.title) }, uniquingKeysWith: { first, _ in first }))
    }

    private static func date(_ ms: Int64) -> Date { Date(timeIntervalSince1970: Double(ms) / 1000) }

    private var rows: [TaskRow] {
        if let stage {
            let tickets = column?.pipeline.first { $0.stage == stage }?.unattended ?? []
            return tickets.map {
                TaskRow(id: $0.task, key: $0.key, title: $0.title, status: .inProgress, statusSince: Self.date($0.since))
            }
        }
        guard let footer else {
            return connection.boards[place.workspace]?.columns.first { $0.status == status }?.rows ?? []
        }
        let board = connection.boards[place.workspace]?.rows ?? []
        let waiting = column?.waiting ?? []
        let ids = Set(waiting.map(\.task))
        switch footer {
        case .waiting:
            // The plan's own tickets, with the board's row where it has one.
            return waiting.map { ticket in
                board.first { $0.id == ticket.task }
                    ?? TaskRow(
                        id: ticket.task, key: ticket.key, title: ticket.title, status: .todo, statusSince: Self.date(ticket.since))
            }
        case .notStarted: return board.filter { [.backlog, .todo].contains($0.status) && !ids.contains($0.id) }
        case .done: return board.filter { $0.status == .done }
        }
    }

    private struct Block: Identifiable {
        let id: String
        let title: String
        let rows: [TaskRow]
    }

    /// A finished status's rows by period; a footer list's in one block.
    private func blocks(_ found: [TaskRow], now: Date) -> [Block] {
        if footer == .waiting || footer == .notStarted || stage != nil {
            return found.isEmpty ? [] : [Block(id: "all", title: title, rows: found)]
        }
        return BoardHistory.groups(found, now: now).map { Block(id: $0.id, title: $0.period.title, rows: $0.rows) }
    }

    private var title: String {
        if let stage { return stage.title }
        switch footer {
        case .waiting: return "Waiting for a Builder"
        case .notStarted: return "Not Started"
        default: return BoardHistory.title(status)
        }
    }

    var body: some View {
        let all = rows
        let found = BoardHistory.filter(all, query: query, area: area)
        BoardTick { now in
            List {
                let areas = BoardHistory.areas(all)
                if areas.count > 1 {
                    Section {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: PaneMetrics.tight) {
                                ForEach(areas, id: \.self) { chip($0) }
                            }
                        }
                        .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
                    }
                }
                ForEach(blocks(found, now: now)) { group in
                    Section {
                        ForEach(group.rows) { row in
                            Button { navigator?.open(.task(place, task: row.id)) } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.step) {
                                        Text(row.key)
                                            .font(.system(.caption, design: .monospaced))
                                            .foregroundStyle(.secondary)
                                        Text(row.title)
                                            .font(.subheadline)
                                            .lineLimit(2)
                                    }
                                    Text(subtitle(row, now: now))
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                }
                                .contentShape(.rect)
                            }
                            .buttonStyle(.plain)
                            // A long press previews the card, with Open (ov-299).
                            .taskKeyCard(row.key, speaksTitle: false)
                            .accessibilityElement(children: .combine)
                            .accessibilityIdentifier("history-row-\(row.key)")
                        }
                    } header: {
                        HStack {
                            Text(group.title)
                            Spacer()
                            Text("\(group.rows.count)")
                                .monospacedDigit()
                                .foregroundStyle(.tertiary)
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .overlay {
                if found.isEmpty {
                    if all.isEmpty {
                        ContentUnavailableView("Nothing Here Yet", systemImage: "checklist")
                    } else {
                        ContentUnavailableView.search(text: query)
                    }
                }
            }
        }
        .searchable(text: $query, prompt: "Keys and titles")
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        // Its rows' keys preview their tasks (ov-299).
        .environment(\.taskKeyLinker, connection.taskKeyLinker(navigator))
        .accessibilityIdentifier("board-history")
    }

    /// Where a list's row stands: a footer or stage list says since when.
    private func subtitle(_ row: TaskRow, now: Date) -> String {
        if let stage {
            return row.statusSince.timeIntervalSince1970 > 0
                ? "\(stage.title) since \(BoardHistory.landed(row, now: now))" : stage.title
        }
        if footer == .waiting || footer == .notStarted { return "\(row.status.title) since \(BoardHistory.landed(row, now: now))" }
        return BoardHistory.landed(row, now: now)
    }

    private func chip(_ name: String) -> some View {
        let on = area == name
        return Button { area = on ? nil : name } label: {
            Text(name)
                .font(.footnote.weight(on ? .semibold : .regular))
                .foregroundStyle(on ? Color.white : Color.primary)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(on ? Color.accentColor : Fill.inset(), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
        .accessibilityIdentifier("history-area-\(name)")
    }
}
