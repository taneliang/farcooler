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

    @State private var query = ""
    @State private var area: String?
    @Environment(\.phoneNavigator) private var navigator

    private var rows: [TaskRow] {
        guard let footer else {
            return connection.boards[place.workspace]?.columns.first { $0.status == status }?.rows ?? []
        }
        let board = connection.boards[place.workspace]?.rows ?? []
        var waiting: [PlanTicketLine] = []
        if case .loaded(let plan)? = connection.plans.state(place.workspace) {
            var seen = Set<String>()
            waiting = plan.lines.filter { $0.stage == .waiting && seen.insert($0.task).inserted }
        }
        let ids = Set(waiting.map(\.task))
        switch footer {
        case .waiting:
            // The plan's own lines, with the board's row where it has one.
            return waiting.compactMap { line in
                let row = board.first { $0.id == line.task }
                if row?.status == .cancelled { return nil }
                return row ?? TaskRow(
                    id: line.task, key: line.key, title: line.heading, status: .todo,
                    statusSince: Date(timeIntervalSince1970: Double(line.stageSince) / 1000))
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
        if footer == .waiting || footer == .notStarted {
            return found.isEmpty ? [] : [Block(id: "all", title: title, rows: found)]
        }
        return BoardHistory.groups(found, now: now).map { Block(id: $0.id, title: $0.period.title, rows: $0.rows) }
    }

    private var title: String {
        switch footer {
        case .waiting: "Waiting for a Builder"
        case .notStarted: "Not Started"
        default: BoardHistory.title(status)
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
                                    Text(footer == .waiting || footer == .notStarted
                                        ? "\(row.status.title) since \(BoardHistory.landed(row, now: now))" : BoardHistory.landed(row, now: now))
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
