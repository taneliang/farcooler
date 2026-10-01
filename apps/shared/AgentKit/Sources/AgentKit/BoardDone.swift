import Foundation

/// What the Done column shows: the work finished lately, newest first.
///
/// Until this rule a board drew every task ever finished, in the order the
/// runner listed them (oldest filed first), so a board with a month of work
/// opened on the oldest cards and the ones just finished were at the bottom
/// of a long column nobody scrolled. The rule is one function here, shared by
/// the Mac, the iPhone and (as a twin in Kotlin) Android, so a card is in
/// Done's short view on all three or none.
///
/// Canceled is not Done and is not shaped by this rule: it is its own status
/// and keeps its own column and section.
public enum BoardDone {
    /// A card finished within this long is always shown.
    public static let recentWindow: TimeInterval = 7 * 24 * 60 * 60

    /// A board shows at least this many, however old, so a quiet week doesn't
    /// empty the column.
    public static let minimumShown = 10

    /// The button that reveals the rest. `total` is every done task.
    public static func showAllTitle(total: Int) -> String { "Show All Done (\(total))" }

    /// The done tasks, newest finished first. A tie keeps the runner's order.
    public static func newestFirst(_ rows: [TaskRow]) -> [TaskRow] {
        rows.enumerated()
            .sorted { a, b in
                a.element.statusSince != b.element.statusSince
                    ? a.element.statusSince > b.element.statusSince : a.offset < b.offset
            }
            .map(\.element)
    }

    /// The done tasks to draw: everything finished inside `recentWindow`, or
    /// the newest `minimumShown`, whichever is more. All of them, newest first,
    /// when `showingAll`.
    public static func visible(_ rows: [TaskRow], showingAll: Bool, now: Date) -> [TaskRow] {
        let sorted = newestFirst(rows)
        guard !showingAll else { return sorted }
        let cutoff = now.addingTimeInterval(-recentWindow)
        let recent = sorted.prefix { $0.statusSince >= cutoff }.count
        return Array(sorted.prefix(max(recent, minimumShown)))
    }
}

extension TaskBoardColumn {
    /// The rows this column draws. Only Done is shortened; every other status
    /// draws all of its rows in the runner's order.
    public func visibleRows(showingAllDone: Bool, now: Date) -> [TaskRow] {
        status == .done
            ? BoardDone.visible(rows, showingAll: showingAllDone, now: now) : rows
    }

    /// Whether Done is hiding anything: the button's reason to exist.
    public func hidesDone(showingAllDone: Bool, now: Date) -> Bool {
        status == .done && !showingAllDone
            && visibleRows(showingAllDone: false, now: now).count < rows.count
    }
}
