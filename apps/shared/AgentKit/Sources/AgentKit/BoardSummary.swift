import Foundation

/// What changed on a board: what's unread (ov-104), or what happened in the
/// last hour or today.
///
/// Built from what the board already holds, so it costs nothing to keep
/// current. A task's `statusSince` says when it finished or moved to Needs
/// Decision or In Review (the status it is in now, entered then), and
/// `createdAt` says when it was filed. Notes live in a task's record, which
/// `task list` doesn't carry; `noteCandidates` names the few tasks whose
/// record is worth reading, and `make` takes what was read.
public struct BoardSummary: Equatable, Sendable {
    /// What the summary covers.
    public enum Period: String, CaseIterable, Sendable, Hashable, Identifiable {
        /// Everything not read yet: an item stays until its ticket is opened.
        case unread
        /// A plain time window: reading changes nothing in it.
        case lastHour
        case today

        public var id: String { rawValue }

        public var title: String {
            switch self {
            case .unread: return "Unread"
            case .lastHour: return "Last Hour"
            case .today: return "Today"
            }
        }
    }

    /// Which items a summary takes: the unread ones, or those since a moment.
    public enum Window: Equatable, Sendable {
        case unread(BoardReads)
        case since(Date)

        /// Whether something that happened on `taskID` at `at` is in it.
        public func includes(_ taskID: String, at: Date) -> Bool {
            switch self {
            case .unread(let reads): return reads.isUnread(taskID, at: at)
            case .since(let start): return at >= start
            }
        }
    }

    /// The window `period` is, now.
    public static func window(
        _ period: Period, reads: BoardReads, now: Date, calendar: Calendar = .current
    ) -> Window {
        switch period {
        case .unread: return .unread(reads)
        case .lastHour: return .since(now.addingTimeInterval(-60 * 60))
        case .today: return .since(calendar.startOfDay(for: now))
        }
    }

    /// One line of the summary, and the task it opens.
    ///
    /// `id` is the ticket's and what happened to it ("<task>/done"), so a
    /// list keyed by it animates an item in, out and along rather than
    /// redrawing it.
    public struct Item: Equatable, Sendable, Identifiable {
        public var id: String
        public var taskID: String
        public var key: String
        public var title: String
        /// What happened, when the title alone doesn't say: a status word.
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

    /// One ticket's notes (ov-104): its newest, whole, and how many older
    /// ones it has in the summary too.
    ///
    /// Keyed by the ticket (`id`), so a newer note replaces the entry in
    /// place; `noteID` is the newest note's, what a new arrival is told by.
    public struct Activity: Equatable, Sendable, Identifiable {
        public var id: String { "\(taskID)/activity" }
        public var taskID: String
        public var key: String
        public var title: String
        public var noteID: String
        /// Its kind's word: Decision, Finding, Comment.
        public var kind: TaskNoteKind
        /// Its body on one run of text, for two wrapped lines.
        public var text: String
        public var at: Date
        /// The ticket's older notes in the summary.
        public var more: Int

        public init(
            taskID: String, key: String, title: String, noteID: String, kind: TaskNoteKind, text: String,
            at: Date, more: Int
        ) {
            self.taskID = taskID
            self.key = key
            self.title = title
            self.noteID = noteID
            self.kind = kind
            self.text = text
            self.at = at
            self.more = more
        }

        /// "+2 more", or nil with none.
        public var moreLine: String? { more > 0 ? "+\(more) more" : nil }
    }

    /// Tasks that reached Done in the window.
    public var finished: [Item]
    /// Tasks now in Needs Decision or In Review that got there in the window.
    public var moved: [Item]
    /// Tasks filed in the window and not listed above.
    public var created: [Item]
    /// Notes written in the window, one entry per ticket.
    public var activity: [Activity]

    public init(finished: [Item] = [], moved: [Item] = [], created: [Item] = [], activity: [Activity] = []) {
        self.finished = finished
        self.moved = moved
        self.created = created
        self.activity = activity
    }

    public var isEmpty: Bool { finished.isEmpty && moved.isEmpty && created.isEmpty && activity.isEmpty }

    /// How many lines it counts: every item and every ticket with activity.
    public var count: Int { finished.count + moved.count + created.count + activity.count }

    /// Only what's about tickets `keep` keeps: the navigator's filter.
    public func filtered(_ keep: (String) -> Bool) -> BoardSummary {
        BoardSummary(
            finished: finished.filter { keep($0.taskID) }, moved: moved.filter { keep($0.taskID) },
            created: created.filter { keep($0.taskID) }, activity: activity.filter { keep($0.taskID) })
    }

    /// How many lines of one group the strip draws before saying "and N more".
    public static let groupLimit = 5

    /// A group cut to `groupLimit`, and how many it left out.
    public static func capped<T>(_ items: [T], limit: Int = groupLimit) -> (shown: [T], more: Int) {
        (Array(items.prefix(limit)), max(0, items.count - limit))
    }

    /// What the strip says when it has nothing to list.
    public static func nothing(in period: Period) -> String {
        switch period {
        case .unread: return "You’re all caught up."
        case .lastHour: return "Nothing new in the last hour."
        case .today: return "Nothing new today."
        }
    }

    /// The note kinds Activity lists: every one a person or an agent writes.
    /// A move and a filing are the Finished and New groups' already.
    public static func listed(_ kind: TaskNoteKind) -> Bool { !kind.isMachineWritten }

    /// The tasks whose records are worth reading for notes: those that moved
    /// inside `window`, most recent first, at most `limit`. A card nothing
    /// touched can't have a new note.
    public static func noteCandidates(rows: [TaskRow], window: Window, limit: Int = 10) -> [TaskRow] {
        rows.filter { $0.status != .cancelled && window.includes($0.id, at: $0.lastMoved) }
            .sorted { $0.lastMoved > $1.lastMoved }
            .prefix(limit).map { $0 }
    }

    /// The summary of `rows` in `window`, with `notes` keyed by task id for
    /// the tasks whose records were read. Newest first in each list.
    public static func make(
        rows: [TaskRow], notes: [String: [TaskNoteRow]] = [:], window: Window
    ) -> BoardSummary {
        var finished: [Item] = []
        var moved: [Item] = []
        var created: [Item] = []
        for row in rows {
            func item(_ what: String, _ detail: String?, at: Date) -> Item {
                Item(id: "\(row.id)/\(what)", taskID: row.id, key: row.key, title: row.title, detail: detail, at: at)
            }
            let changed = window.includes(row.id, at: row.statusSince)
            switch row.status {
            case .done where changed:
                finished.append(item("done", nil, at: row.statusSince))
            case .needsDecision where changed, .inReview where changed:
                moved.append(item(row.status.rawValue, row.status.title, at: row.statusSince))
            case .cancelled:
                break
            default:
                if let filed = row.createdAt, window.includes(row.id, at: filed) {
                    created.append(item("created", nil, at: filed))
                }
            }
        }
        var activity: [Activity] = []
        for row in rows where row.status != .cancelled {
            let written = (notes[row.id] ?? [])
                .filter { listed($0.kind) && window.includes(row.id, at: $0.at) }
                .sorted { $0.at > $1.at }
            guard let newest = written.first else { continue }
            activity.append(
                Activity(
                    taskID: row.id, key: row.key, title: row.title, noteID: newest.id, kind: newest.kind,
                    text: oneRun(newest.body), at: newest.at, more: written.count - 1))
        }
        func newest(_ items: [Item]) -> [Item] { items.sorted { $0.at > $1.at } }
        return BoardSummary(
            finished: newest(finished), moved: newest(moved), created: newest(created),
            activity: activity.sorted { $0.at > $1.at })
    }

    /// A note's body as one run of text: its lines and runs of spaces as
    /// single spaces, for two wrapped lines.
    public static func oneRun(_ body: String) -> String {
        body.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    /// The closed strip's one line: "3 unread", "2 new today".
    public static func collapsedLine(count: Int, period: Period) -> String {
        switch period {
        case .unread: return count == 0 ? "Nothing unread" : "\(count) unread"
        case .lastHour: return count == 0 ? "Nothing new in the last hour" : "\(count) new in the last hour"
        case .today: return count == 0 ? "Nothing new today" : "\(count) new today"
        }
    }
}

/// When a person last left a workspace's board on this device, as Since
/// Last Visit kept it. Read once now, to start a board's read state where
/// the last visit left off (`DefaultsBoardReads.load`).
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
