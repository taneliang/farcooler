import Foundation

/// The History page (ov-103): every task finished in a status, grouped by
/// when it landed, searchable, and filtered by the area its title names.
public enum BoardHistory {
    /// When a task landed, as the page groups it.
    public enum Period: String, CaseIterable, Sendable, Hashable, Identifiable {
        case today, yesterday, thisWeek, earlier

        public var id: String { rawValue }

        public var title: String { BoardHistory.titles[self] ?? rawValue }
    }

    private static let titles: [Period: String] = {
        let labels: [String] = TaskBoardPresenter.ask("period_titles") ?? []
        return Dictionary(uniqueKeysWithValues: zip(Period.allCases, labels))
    }()

    public struct Group: Equatable, Sendable, Identifiable {
        public var period: Period
        public var rows: [TaskRow]
        public var id: String { period.rawValue }
    }

    /// The page's title: "Done", "Canceled".
    public static func title(_ status: TaskStatus) -> String { status.title }

    private static func boundaries(_ now: Date, _ calendar: Calendar) -> [String: Double] {
        let today = calendar.startOfDay(for: now)
        let day = calendar.dateInterval(of: .day, for: now)
        let yesterday = calendar.date(byAdding: .day, value: -1, to: now).map { calendar.startOfDay(for: $0) }
        let week = calendar.dateInterval(of: .weekOfYear, for: now)
        return [
            "today": today, "tomorrow": day?.end, "yesterday": yesterday,
            "week_start": week?.start, "week_end": week?.end,
        ].compactMapValues { $0?.timeIntervalSinceReferenceDate }
    }

    public static func period(of date: Date, now: Date, calendar: Calendar = .current) -> Period {
        let index: Int = TaskBoardPresenter.ask("history_period", [
            "at": date.timeIntervalSinceReferenceDate, "calendar": boundaries(now, calendar),
        ]) ?? Period.allCases.count - 1
        return Period.allCases[index]
    }

    public static func groups(_ rows: [TaskRow], now: Date, calendar: Calendar = .current) -> [Group] {
        let groups: [[Int]] = TaskBoardPresenter.ask("history_groups", [
            "stamps": rows.map { $0.statusSince.timeIntervalSinceReferenceDate },
            "calendar": boundaries(now, calendar),
        ]) ?? []
        return zip(Period.allCases, groups).compactMap { period, indices in
            indices.isEmpty ? nil : Group(period: period, rows: indices.map { rows[$0] })
        }
    }

    public static func area(of title: String) -> String? {
        TaskBoardPresenter.ask("history_area", ["title": title])
    }

    public static func areas(_ rows: [TaskRow]) -> [String] {
        TaskBoardPresenter.ask("history_areas", ["titles": rows.map(\.title)]) ?? []
    }

    public static func filter(
        _ rows: [TaskRow], query: String, area: String? = nil, noteHits: Set<String> = []
    ) -> [TaskRow] {
        let indices = BoardFilter.indices(rows.map { ["id": $0.id, "key": $0.key, "title": $0.title] },
            query: query, area: area, noteHits: noteHits)
        return indices.map { rows[$0] }
    }

    /// When a row landed, under its period's heading: the time today and
    /// yesterday, the weekday and time this week, the date before that.
    public static func landed(
        _ row: TaskRow, now: Date, calendar: Calendar = .current, locale: Locale = .current
    ) -> String {
        var style: Date.FormatStyle
        switch period(of: row.statusSince, now: now, calendar: calendar) {
        case .today, .yesterday: style = Date.FormatStyle(date: .omitted, time: .shortened)
        case .thisWeek: style = Date.FormatStyle().weekday(.abbreviated).hour().minute()
        case .earlier:
            let sameYear = calendar.component(.year, from: row.statusSince) == calendar.component(.year, from: now)
            style = sameYear ? Date.FormatStyle().month(.abbreviated).day() : Date.FormatStyle().month(.abbreviated).day().year()
        }
        style.calendar = calendar
        style.timeZone = calendar.timeZone
        style.locale = locale
        return row.statusSince.formatted(style)
    }
}

/// The navigator's filter (⌘F, ov-103): a task matches when its key or title
/// carries every word typed, ignoring case and accents.
public enum BoardFilter {
    public static func isEmpty(_ query: String) -> Bool {
        TaskBoardPresenter.ask("search_empty", ["query": query]) ?? query.isEmpty
    }

    fileprivate static func indices(
        _ rows: [[String: String]], query: String, area: String? = nil, noteHits: Set<String> = []
    ) -> [Int] {
        var fields: [String: Any] = ["rows": rows, "query": query, "note_hits": Array(noteHits)]
        fields["area"] = area
        return TaskBoardPresenter.ask("search", fields) ?? []
    }

    public static func matches(_ row: TaskRow, _ query: String) -> Bool {
        matches(key: row.key, title: row.title, query)
    }

    public static func matches(key: String, title: String, _ query: String) -> Bool {
        !indices([["key": key, "title": title]], query: query).isEmpty
    }

    public static func narrowed(_ board: TaskBoardModel, _ query: String) -> TaskBoardModel {
        guard !isEmpty(query) else { return board }
        return TaskBoardModel(
            columns: board.columns.map { TaskBoardColumn(status: $0.status, rows: BoardHistory.filter($0.rows, query: query)) },
            unreadable: indices(board.unreadable.map { ["key": $0.key, "title": $0.title] }, query: query)
                .map { board.unreadable[$0] })
    }
}

/// Which items in a list just arrived (ov-104): what's drawn with a brief
/// accent highlight as it slides in. By identity, so an item that moved or
/// changed isn't new, and on a first draw (`old` nil) nothing is.
public enum BoardArrivals {
    public static func new(old: [String]?, now: [String]) -> Set<String> {
        guard let old else { return [] }
        return Set(now).subtracting(old)
    }

    /// Which items arrived or changed (ov-298): `new`'s rule, by identity,
    /// and also each item still there whose `signature` (what its row shows,
    /// a ticket's stage or a theme's count, say) moved. On a first draw (`old`
    /// nil), none; an item that left is not changed, it's gone.
    public static func changed(old: [String: String]?, now: [String: String]) -> Set<String> {
        guard let old else { return [] }
        return Set(now.keys.filter { old[$0] != now[$0] })
    }
}
