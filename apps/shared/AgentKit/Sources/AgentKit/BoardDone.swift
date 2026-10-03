import Foundation

/// What the Done and Canceled sections show (ov-103), and how long any other
/// section runs before "Show N More".
///
/// Done shows every task finished and still unread (`BoardReads`), however
/// many, plus everything finished today, with a floor of the latest three on
/// a quiet day. The rest are on the History page, behind the "All Done"
/// row. Canceled works the same. The rule is one function here, shared by the
/// Mac and the iPhone, and transcribed for Android, so a task is in Done's
/// short view on all three or none.
public enum BoardDone {
    /// Done shows at least this many, however old.
    public static let floor = 3

    /// The row that opens the History page: "All Done", "All Canceled".
    /// Its count is drawn beside it, as a header's is, never in parentheses.
    public static func historyTitle(_ status: TaskStatus) -> String { "All \(status.title)" }

    /// The finished tasks, newest finished first. A tie keeps the runner's
    /// order.
    public static func newestFirst(_ rows: [TaskRow]) -> [TaskRow] {
        rows.enumerated()
            .sorted { a, b in
                a.element.statusSince != b.element.statusSince
                    ? a.element.statusSince > b.element.statusSince : a.offset < b.offset
            }
            .map(\.element)
    }

    /// The finished tasks to draw, newest first: the unread ones, those
    /// finished today, and at least the newest `floor`.
    public static func shown(
        _ rows: [TaskRow], reads: BoardReads, now: Date, calendar: Calendar = .current
    ) -> [TaskRow] {
        newestFirst(rows).enumerated()
            .filter { index, row in
                index < floor || reads.finishedUnread(row) || calendar.isDate(row.statusSince, inSameDayAs: now)
            }
            .map(\.element)
    }
}

/// How a section in the navigator is cut (ov-103).
public enum BoardSectionCut {
    /// A section other than Done and Canceled shows this many before "Show
    /// N More".
    public static let limit = 10

    /// "Show 4 More".
    public static func showMoreTitle(_ hidden: Int) -> String { "Show \(hidden) More" }
}

extension TaskBoardColumn {
    /// What this section draws, and what it leaves out.
    public struct Cut: Equatable, Sendable {
        public var rows: [TaskRow]
        /// Left out of a long section until "Show N More".
        public var hidden: Int
        /// Done and Canceled: the row to the History page, with every task
        /// in the status.
        public var history: Int?
    }

    /// Every row, in the order the section draws them: Done and Canceled
    /// newest finished first, any other in the runner's order.
    public var orderedRows: [TaskRow] { status.isFinished ? BoardDone.newestFirst(rows) : rows }

    /// The rows this section draws. Done and Canceled by `BoardDone`'s rule,
    /// with the History row; any other the first `limit` until `showingAll`.
    /// A filtered list (`filtering`) shows every match.
    public func cut(
        reads: BoardReads, showingAll: Bool = false, filtering: Bool = false, now: Date,
        calendar: Calendar = .current
    ) -> Cut {
        if status.isFinished {
            let shown = filtering ? BoardDone.newestFirst(rows) : BoardDone.shown(rows, reads: reads, now: now, calendar: calendar)
            return Cut(rows: shown, hidden: 0, history: rows.isEmpty ? nil : rows.count)
        }
        guard !showingAll, !filtering, rows.count > BoardSectionCut.limit else {
            return Cut(rows: rows, hidden: 0, history: nil)
        }
        return Cut(rows: Array(rows.prefix(BoardSectionCut.limit)), hidden: rows.count - BoardSectionCut.limit, history: nil)
    }
}
