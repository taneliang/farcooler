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

    @State private var query = ""
    @State private var area: String?
    @Environment(\.phoneNavigator) private var navigator

    private var rows: [TaskRow] {
        connection.boards[place.workspace]?.columns.first { $0.status == status }?.rows ?? []
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
                ForEach(BoardHistory.groups(found, now: now)) { group in
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
                                    Text(BoardHistory.landed(row, now: now))
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                }
                                .contentShape(.rect)
                            }
                            .buttonStyle(.plain)
                            .accessibilityElement(children: .combine)
                            .accessibilityIdentifier("history-row-\(row.key)")
                        }
                    } header: {
                        HStack {
                            Text(group.period.title)
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
        .navigationTitle(BoardHistory.title(status))
        .navigationBarTitleDisplayMode(.inline)
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
                .background(on ? Color.accentColor : Color.primary.opacity(0.08), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
        .accessibilityIdentifier("history-area-\(name)")
    }
}
