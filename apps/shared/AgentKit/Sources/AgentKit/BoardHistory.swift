import Foundation

/// The History page (ov-103): every task finished in a status, grouped by
/// when it landed, searchable, and filtered by the area its title names.
public enum BoardHistory {
    /// When a task landed, as the page groups it.
    public enum Period: String, CaseIterable, Sendable, Hashable, Identifiable {
        case today, yesterday, thisWeek, earlier

        public var id: String { rawValue }

        public var title: String {
            switch self {
            case .today: return "Today"
            case .yesterday: return "Yesterday"
            case .thisWeek: return "This Week"
            case .earlier: return "Earlier"
            }
        }
    }

    public struct Group: Equatable, Sendable, Identifiable {
        public var period: Period
        public var rows: [TaskRow]
        public var id: String { period.rawValue }
    }

    /// The page's title: "Done", "Canceled".
    public static func title(_ status: TaskStatus) -> String { status.title }

    public static func period(of date: Date, now: Date, calendar: Calendar = .current) -> Period {
        if calendar.isDate(date, inSameDayAs: now) { return .today }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: now),
            calendar.isDate(date, inSameDayAs: yesterday)
        {
            return .yesterday
        }
        if let week = calendar.dateInterval(of: .weekOfYear, for: now), week.contains(date) { return .thisWeek }
        return .earlier
    }

    /// `rows`, newest landed first, in their periods, the empty ones left
    /// out.
    public static func groups(_ rows: [TaskRow], now: Date, calendar: Calendar = .current) -> [Group] {
        let sorted = BoardDone.newestFirst(rows)
        return Period.allCases.compactMap { period in
            let inIt = sorted.filter { self.period(of: $0.statusSince, now: now, calendar: calendar) == period }
            return inIt.isEmpty ? nil : Group(period: period, rows: inIt)
        }
    }

    /// The area a title names, `<Area>: <outcome>` (the manager's
    /// convention): "Mac" of "Mac: diff viewer…". Nil for a title without
    /// one, or with a prefix too long to be an area.
    public static func area(of title: String) -> String? {
        guard let colon = title.range(of: ": ") else { return nil }
        let area = title[..<colon.lowerBound].trimmingCharacters(in: .whitespaces)
        guard !area.isEmpty, area.count <= 16, !area.contains(where: { ".,;!?()".contains($0) }) else { return nil }
        return area
    }

    /// The areas `rows` name, most used first, then by name: the chips.
    public static func areas(_ rows: [TaskRow]) -> [String] {
        var counts: [String: Int] = [:]
        for row in rows { if let area = area(of: row.title) { counts[area, default: 0] += 1 } }
        return counts.sorted { $0.value != $1.value ? $0.value > $1.value : $0.key < $1.key }.map(\.key)
    }

    /// `rows` in `area` (nil for any) whose key or title carries every word
    /// of `query`, or whose notes do (`noteHits`, task ids the runner's note
    /// search answered).
    public static func filter(
        _ rows: [TaskRow], query: String, area: String? = nil, noteHits: Set<String> = []
    ) -> [TaskRow] {
        rows.filter { row in
            (area == nil || self.area(of: row.title) == area)
                && (BoardFilter.matches(row, query) || (!BoardFilter.isEmpty(query) && noteHits.contains(row.id)))
        }
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
    public static func isEmpty(_ query: String) -> Bool { words(query).isEmpty }

    static func words(_ query: String) -> [String] {
        query.split(whereSeparator: \.isWhitespace).map(String.init)
    }

    public static func matches(_ row: TaskRow, _ query: String) -> Bool {
        matches(key: row.key, title: row.title, query)
    }

    public static func matches(key: String, title: String, _ query: String) -> Bool {
        let haystack = key + " " + title
        return words(query).allSatisfy {
            haystack.range(of: $0, options: [.caseInsensitive, .diacriticInsensitive]) != nil
        }
    }

    /// `board` with only the tasks that match: every section narrowed, the
    /// empty ones kept as headers reading 0.
    public static func narrowed(_ board: TaskBoardModel, _ query: String) -> TaskBoardModel {
        guard !isEmpty(query) else { return board }
        return TaskBoardModel(
            columns: board.columns.map { TaskBoardColumn(status: $0.status, rows: $0.rows.filter { matches($0, query) }) },
            unreadable: board.unreadable.filter { matches(key: $0.key, title: $0.title, query) })
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
    /// a lane's state or a theme's count, say) moved. On a first draw (`old`
    /// nil), none; an item that left is not changed, it's gone.
    public static func changed(old: [String: String]?, now: [String: String]) -> Set<String> {
        guard let old else { return [] }
        return Set(now.keys.filter { old[$0] != now[$0] })
    }
}
