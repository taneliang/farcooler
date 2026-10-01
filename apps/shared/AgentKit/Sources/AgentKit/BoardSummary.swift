import Foundation

/// "Since you were last here": what changed on a board over a period.
///
/// Built from what the board already holds, so it costs nothing to keep
/// current. A task's `statusSince` says when it finished or moved to Needs
/// Decision or In Review (the status it is in now, entered then), and
/// `createdAt` says when it was filed. Decisions and findings live in a task's
/// record, which `task list` doesn't carry; `noteCandidates` names the few
/// tasks whose record is worth reading, and `make` takes what was read.
public struct BoardSummary: Equatable, Sendable {
    /// The span the summary covers.
    public enum Period: String, CaseIterable, Sendable, Hashable, Identifiable {
        case sinceLastVisit
        case lastHour
        case today

        public var id: String { rawValue }

        public var title: String {
            switch self {
            case .sinceLastVisit: return "Since Last Visit"
            case .lastHour: return "Last Hour"
            case .today: return "Today"
            }
        }
    }

    /// One line of the summary, and the task it opens.
    public struct Item: Equatable, Sendable, Identifiable {
        public var id: String
        public var taskID: String
        public var key: String
        public var title: String
        /// What happened, when the title alone doesn't say: a status word, or
        /// a decision's first line.
        public var detail: String?
        public var at: Date

        public init(
            id: String, taskID: String, key: String, title: String, detail: String? = nil, at: Date
        ) {
            self.id = id
            self.taskID = taskID
            self.key = key
            self.title = title
            self.detail = detail
            self.at = at
        }
    }

    /// Tasks that reached Done in the period.
    public var finished: [Item]
    /// Tasks now in Needs Decision or In Review that got there in the period.
    public var moved: [Item]
    /// Tasks filed in the period and not listed above.
    public var created: [Item]
    /// Decisions and findings written in the period.
    public var notes: [Item]

    public init(finished: [Item] = [], moved: [Item] = [], created: [Item] = [], notes: [Item] = []) {
        self.finished = finished
        self.moved = moved
        self.created = created
        self.notes = notes
    }

    public var isEmpty: Bool { finished.isEmpty && moved.isEmpty && created.isEmpty && notes.isEmpty }

    /// How many lines of one group the strip draws before saying "and N more".
    public static let groupLimit = 5

    /// A group cut to `groupLimit`, and how many it left out.
    public static func capped(_ items: [Item], limit: Int = groupLimit) -> (shown: [Item], more: Int) {
        (Array(items.prefix(limit)), max(0, items.count - limit))
    }

    /// What the strip says when it has nothing to list.
    public static let nothingNew = "Nothing new since you were last here."

    /// The moment a period starts. `lastVisit` is nil on a board never visited
    /// on this Mac, which reads as the last day: a first look should show
    /// something recent, not the whole board's history.
    public static func start(
        of period: Period, lastVisit: Date?, now: Date, calendar: Calendar = .current
    ) -> Date {
        switch period {
        case .sinceLastVisit: return lastVisit ?? now.addingTimeInterval(-24 * 60 * 60)
        case .lastHour: return now.addingTimeInterval(-60 * 60)
        case .today: return calendar.startOfDay(for: now)
        }
    }

    /// The tasks whose records are worth reading for decisions and findings:
    /// those that moved at or after `since`, most recent first, at most
    /// `limit`. A card nothing touched can't have a new note.
    public static func noteCandidates(rows: [TaskRow], since: Date, limit: Int = 10) -> [TaskRow] {
        rows.filter { $0.status != .cancelled && ($0.updatedAt ?? $0.statusSince) >= since }
            .sorted { ($0.updatedAt ?? $0.statusSince) > ($1.updatedAt ?? $1.statusSince) }
            .prefix(limit).map { $0 }
    }

    /// The summary of `rows` since `since`, with `notes` keyed by task id for
    /// the tasks whose records were read. Newest first in each list.
    public static func make(
        rows: [TaskRow], notes: [String: [TaskNoteRow]] = [:], since: Date
    ) -> BoardSummary {
        var finished: [Item] = []
        var moved: [Item] = []
        var created: [Item] = []
        for row in rows {
            func item(_ detail: String?, at: Date) -> Item {
                Item(id: "\(row.id)/\(row.status.rawValue)", taskID: row.id, key: row.key,
                     title: row.title, detail: detail, at: at)
            }
            switch row.status {
            case .done where row.statusSince >= since:
                finished.append(item(nil, at: row.statusSince))
            case .needsDecision where row.statusSince >= since,
                .inReview where row.statusSince >= since:
                moved.append(item(row.status.title, at: row.statusSince))
            case .cancelled:
                break
            default:
                if let filed = row.createdAt, filed >= since {
                    created.append(item(nil, at: filed))
                }
            }
        }
        var written: [Item] = []
        for row in rows where row.status != .cancelled {
            for note in notes[row.id] ?? []
            where note.at >= since && (note.kind == .decision || note.kind == .finding) {
                let line = note.body.split(whereSeparator: \.isNewline).first.map(String.init) ?? ""
                written.append(
                    Item(
                        id: "\(row.id)/note/\(note.id)", taskID: row.id, key: row.key,
                        title: row.title,
                        detail: line.isEmpty ? note.kind.title : "\(note.kind.title): \(line)",
                        at: note.at))
            }
        }
        func newest(_ items: [Item]) -> [Item] { items.sorted { $0.at > $1.at } }
        return BoardSummary(
            finished: newest(finished), moved: newest(moved), created: newest(created),
            notes: newest(written))
    }
}

/// When a person last used a workspace's board and orchestrator on this
/// device, kept per runner and workspace like the board's form.
public enum BoardVisit {
    public static func key(host: String, workspace: String) -> String {
        "board.lastVisit.\(host).\(workspace)"
    }

    public static func read(
        host: String, workspace: String, from defaults: UserDefaults = .standard
    ) -> Date? {
        let seconds = defaults.double(forKey: key(host: host, workspace: workspace))
        return seconds > 0 ? Date(timeIntervalSince1970: seconds) : nil
    }

    public static func write(
        _ date: Date, host: String, workspace: String, in defaults: UserDefaults = .standard
    ) {
        defaults.set(date.timeIntervalSince1970, forKey: key(host: host, workspace: workspace))
    }
}
