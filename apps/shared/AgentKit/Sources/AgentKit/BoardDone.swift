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
    public static var floor: Int { TaskBoardPresenter.settings?.doneFloor ?? 3 }

    /// The row that opens the History page: "All Done", "All Canceled".
    /// Its count is drawn beside it, as a header's is, never in parentheses.
    public static func historyTitle(_ status: TaskStatus) -> String {
        TaskBoardPresenter.ask("history_title", ["status": status.rawValue]) ?? status.title
    }

    public static func newestFirst(_ rows: [TaskRow]) -> [TaskRow] {
        BoardSectionCut.project(rows, finished: true, filtering: true).rows
    }

    public static func shown(
        _ rows: [TaskRow], reads: BoardReads, keeping: String? = nil, now: Date, calendar: Calendar = .current
    ) -> [TaskRow] {
        BoardSectionCut.project(rows, finished: true, keeping: keeping) {
            reads.finishedUnread($0) || calendar.isDate($0.statusSince, inSameDayAs: now)
        }.rows
    }
}

/// How a section in the navigator is cut (ov-103).
public enum BoardSectionCut {
    /// A section other than Done and Canceled shows this many before "Show
    /// N More".
    public static var limit: Int { TaskBoardPresenter.settings?.sectionLimit ?? 10 }

    /// "Show 4 More".
    public static func showMoreTitle(_ hidden: Int) -> String {
        TaskBoardPresenter.ask("show_more", ["count": hidden]) ?? ""
    }

    static func project(
        _ rows: [TaskRow], finished: Bool, keeping: String? = nil, showingAll: Bool = false,
        filtering: Bool = false, recent: (TaskRow) -> Bool = { _ in false }
    ) -> TaskBoardColumn.Cut {
        struct View: Decodable {
            let indices: [Int]
            let hidden: Int
            let history: Int?
        }
        var fields: [String: Any] = [
            "rows": rows.map { row -> [String: Any] in
                ["id": row.id, "since": row.statusSince.timeIntervalSinceReferenceDate, "recent": recent(row)]
            },
            "finished": finished, "all": showingAll, "filtering": filtering,
        ]
        fields["keeping"] = keeping
        guard let view: View = TaskBoardPresenter.ask("cut", fields) else {
            return TaskBoardColumn.Cut(rows: rows, hidden: 0, history: nil)
        }
        return TaskBoardColumn.Cut(rows: view.indices.map { rows[$0] }, hidden: view.hidden, history: view.history)
    }
}

extension TaskBoardColumn {
    /// What this section draws, and what it leaves out.
    public struct Cut: Equatable, Sendable {
        public var rows: [TaskRow]
        /// Left out of a long section until "Show N More".
        public var hidden: Int
        /// Done and Canceled: the row to the History page, with every task
        /// in the status, while some aren't drawn here.
        public var history: Int?
    }

    /// Every row, in the order the section draws them: Done and Canceled
    /// newest finished first, any other in the runner's order.
    public var orderedRows: [TaskRow] { status.isFinished ? BoardDone.newestFirst(rows) : rows }

    /// The rows this section draws. Done and Canceled by `BoardDone`'s rule,
    /// with the History row while it leaves some out; any other the first
    /// `limit` until `showingAll`. A filtered list (`filtering`) shows every
    /// match. `keeping`, the task selected, is never cut.
    public func cut(
        reads: BoardReads, keeping: String? = nil, showingAll: Bool = false, filtering: Bool = false, now: Date,
        calendar: Calendar = .current
    ) -> Cut {
        BoardSectionCut.project(rows, finished: status.isFinished, keeping: keeping, showingAll: showingAll, filtering: filtering) {
            status.isFinished && (reads.finishedUnread($0) || calendar.isDate($0.statusSince, inSameDayAs: now))
        }
    }
}
