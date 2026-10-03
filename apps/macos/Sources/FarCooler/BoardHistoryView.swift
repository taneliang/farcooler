import AgentKit
import SwiftUI

/// The History page (ov-103): every task finished in a status, opened in the
/// main area from the navigator's "All Done" row. Grouped Today, Yesterday,
/// This Week and Earlier; searchable by key and title here, and by note text
/// through the runner's record (`task search`); narrowed by the areas the
/// titles name (`<Area>: <outcome>`). Each row says when it landed, and a
/// click opens its ticket.
struct BoardHistoryView: View {
    @ObservedObject var store: TaskBoardStore
    let status: TaskStatus
    let onOpen: (TaskRow) -> Void

    @State private var query = ""
    @State private var area: String?
    /// The tasks whose notes carry a query, as the runner last answered,
    /// and which query that was.
    @State private var noteHits = NoteHits()
    @FocusState private var searchFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var rows: [TaskRow] { store.board.columns.first { $0.status == status }?.rows ?? [] }

    var body: some View {
        let all = rows
        let found = BoardHistory.filter(all, query: query, area: area, noteHits: noteHits.ids(for: query))
        BoardTick { now in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2 * ColumnGrid.rhythm) {
                    header(total: all.count)
                    chips(BoardHistory.areas(all))
                    if found.isEmpty {
                        Text(all.isEmpty ? "Nothing here yet." : "No tasks match.")
                            .font(.system(size: WorkspaceStyle.PaneText.body))
                            .foregroundStyle(.secondary)
                            .padding(.leading, ColumnGrid.step)
                    }
                    ForEach(BoardHistory.groups(found, now: now)) { group in
                        VStack(alignment: .leading, spacing: 2) {
                            GroupHeader(title: group.period.title, count: group.rows.count)
                                .padding(.leading, ColumnGrid.step)
                            ForEach(group.rows) { row in
                                CompactTaskRow(key: row.key, title: row.title) {
                                    Text(BoardHistory.landed(row, now: now))
                                        .font(.system(size: WorkspaceStyle.PaneText.minimum))
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                }
                                .contentShape(Rectangle())
                                .onTapGesture { onOpen(row) }
                                .accessibilityElement(children: .combine)
                                .accessibilityAddTraits(.isButton)
                                .accessibilityIdentifier("history-row-\(row.key)")
                            }
                        }
                    }
                }
                .animation(BoardMotion.list(reduceMotion: reduceMotion), value: found.map(\.id))
                .frame(maxWidth: 720, alignment: .leading)
                .padding(.horizontal, ColumnGrid.a)
                .padding(.vertical, 2 * ColumnGrid.rhythm)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .environment(\.taskKeyWidth, TaskKeyColumn.width(for: all.map(\.key)))
        .background(WorkspaceStyle.document)
        // The runner's note search, a moment after typing stops. It matches
        // the phrase as typed, where keys and titles match every word: the
        // record's search is literal (`task search`). Its hits count only
        // for the query they answered (`NoteHits`).
        .task(id: query) {
            guard !BoardFilter.isEmpty(query) else { return }
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            let hits = await store.noteHits(query)
            if !Task.isCancelled { noteHits = NoteHits(query: query, ids: hits) }
        }
        .onAppear { searchFocused = true }
        .accessibilityIdentifier("board-history")
    }

    private func header(total: Int) -> some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
            HStack(alignment: .firstTextBaseline) {
                Text(BoardHistory.title(status))
                    .font(.system(size: 17, weight: .semibold))
                Spacer(minLength: SidebarGrid.gap)
                SectionCount(count: total)
            }
            .padding(.leading, ColumnGrid.step)
            NavigatorFilterField(
                text: $query, focused: $searchFocused, placeholder: "Search keys, titles, and notes",
                glyph: "magnifyingglass"
            ) {
                searchFocused = false
            }
            .padding(.horizontal, ColumnGrid.step)
        }
    }

    /// The areas as chips: one chosen narrows the page to it; chosen again,
    /// it lets go.
    @ViewBuilder private func chips(_ areas: [String]) -> some View {
        if areas.count > 1 {
            AreaChips(areas: areas, chosen: $area)
                .padding(.leading, ColumnGrid.step)
        }
    }
}

/// The runner's note-search answer, kept with the query it answered: it
/// counts only for that query, so the last one's hits never hold rows in
/// while the next is being asked (ov-104 review).
struct NoteHits: Equatable {
    var query = ""
    var ids: Set<String> = []

    func ids(for current: String) -> Set<String> { current == query ? ids : [] }
}

/// A row of area chips, wrapping onto as many lines as it takes.
private struct AreaChips: View {
    let areas: [String]
    @Binding var chosen: String?

    var body: some View {
        ChipFlow(spacing: 6) {
            ForEach(areas, id: \.self) { area in
                let on = chosen == area
                Button { chosen = on ? nil : area } label: {
                    Text(area)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: on ? .semibold : .regular))
                        .foregroundStyle(on ? Color.white : Color.primary)
                        .padding(.horizontal, 8)  // grid-exempt: a chip's own inset
                        .padding(.vertical, 3)
                        .background(Capsule().fill(on ? Color.accentColor : Color.primary.opacity(0.07)))
                        .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
                .accessibilityIdentifier("history-area-\(area)")
            }
        }
    }
}

/// Lays its children out left to right, wrapping at the proposed width.
private struct ChipFlow: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        var x: CGFloat = 0
        var y: CGFloat = 0
        var line: CGFloat = 0
        var widest: CGFloat = 0
        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                y += line + spacing
                x = 0
                line = 0
            }
            x += size.width + spacing
            line = max(line, size.height)
            widest = max(widest, x - spacing)
        }
        return CGSize(width: min(widest, width), height: y + line)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX
        var y = bounds.minY
        var line: CGFloat = 0
        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            if x > bounds.minX, x + size.width > bounds.maxX {
                y += line + spacing
                x = bounds.minX
                line = 0
            }
            view.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
            x += size.width + spacing
            line = max(line, size.height)
        }
    }
}
