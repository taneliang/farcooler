import Foundation
import Testing

@testable import AgentKit

// Done shows what was finished lately, newest first (ov-80), and the summary
// says what changed since a moment.

private let now = Date(timeIntervalSince1970: 1_800_000_000)
private let day: TimeInterval = 24 * 60 * 60

private func row(
    _ key: String, _ status: TaskStatus, since ago: TimeInterval, created: TimeInterval? = nil
) -> TaskRow {
    TaskRow(
        id: "id-\(key)", key: key, title: "Title \(key)", status: status,
        statusSince: now.addingTimeInterval(-ago),
        createdAt: created.map { now.addingTimeInterval(-$0) })
}

@Test("Done shows the last 7 days, newest first, and keeps at least 10")
func doneShowsTheLastSevenDaysNewestFirstAndKeepsAtLeastTen() {
    // 12 finished in the last two days, 5 a month ago: all 12 recent, none old.
    var rows = (0..<12).map { row("r\($0)", .done, since: Double($0) * 3600) }
    rows += (0..<5).map { row("old\($0)", .done, since: 30 * day + Double($0)) }
    let shown = BoardDone.visible(rows.reversed(), showingAll: false, now: now)
    #expect(shown.map(\.key) == (0..<12).map { "r\($0)" })

    // One recent card on a quiet board: topped up to 10 with the newest older ones.
    let quiet = [row("a", .done, since: 3600)] + (0..<20).map { row("o\($0)", .done, since: 30 * day + Double($0)) }
    let top = BoardDone.visible(quiet, showingAll: false, now: now)
    #expect(top.count == 10)
    #expect(top.first?.key == "a")
    #expect(top.last?.key == "o8")
}

@Test("Show All reveals every done card, newest first")
func showAllRevealsEveryDoneCardNewestFirst() {
    let rows = (0..<25).map { row("o\($0)", .done, since: 30 * day + Double($0)) }
    #expect(BoardDone.visible(rows, showingAll: true, now: now).count == 25)
    #expect(BoardDone.visible(rows, showingAll: true, now: now).first?.key == "o0")
    #expect(BoardDone.showAllTitle(total: 25) == "Show All Done (25)")
}

@Test("Only Done is shortened; Canceled and the rest draw every row")
func onlyDoneIsShortened() {
    let old = (0..<15).map { row("c\($0)", .cancelled, since: 40 * day) }
    let canceled = TaskBoardColumn(status: .cancelled, rows: old)
    #expect(canceled.visibleRows(showingAllDone: false, now: now).count == 15)
    #expect(!canceled.hidesDone(showingAllDone: false, now: now))
    let done = TaskBoardColumn(status: .done, rows: (0..<15).map { row("d\($0)", .done, since: 40 * day + Double($0)) })
    #expect(done.visibleRows(showingAllDone: false, now: now).count == 10)
    #expect(done.hidesDone(showingAllDone: false, now: now))
    #expect(!done.hidesDone(showingAllDone: true, now: now))
}

@Test("The summary lists finished, moved, new and noted work since the start")
func theSummaryListsWhatChanged() {
    let since = now.addingTimeInterval(-3600)
    let rows = [
        row("fin", .done, since: 600),
        row("oldfin", .done, since: 7200),
        row("nd", .needsDecision, since: 300),
        row("rev", .inReview, since: 100),
        row("oldrev", .inReview, since: 5000),
        row("new", .todo, since: 200, created: 200),
        row("oldnew", .todo, since: 9000, created: 9000),
        row("gone", .cancelled, since: 100, created: 100),
    ]
    let notes = [
        "id-oldnew": [
            TaskNoteRow(id: "n1", kind: .decision, actor: "user", at: now.addingTimeInterval(-60), body: "Use SQLite\nbecause"),
            TaskNoteRow(id: "n2", kind: .comment, actor: "user", at: now.addingTimeInterval(-60), body: "meh"),
            TaskNoteRow(id: "n3", kind: .finding, actor: "user", at: now.addingTimeInterval(-9000), body: "old"),
        ]
    ]
    let summary = BoardSummary.make(rows: rows, notes: notes, since: since)
    #expect(summary.finished.map(\.key) == ["fin"])
    #expect(summary.moved.map(\.key) == ["rev", "nd"])
    #expect(summary.moved.first?.detail == "In Review")
    #expect(summary.created.map(\.key) == ["new"])
    #expect(summary.notes.map(\.detail) == ["Decision: Use SQLite"])
    #expect(summary.notes.first?.taskID == "id-oldnew")
    #expect(!summary.isEmpty)
    #expect(BoardSummary.make(rows: rows, since: now).isEmpty)
    #expect(BoardSummary.nothingNew == "Nothing new since you were last here.")
}

@Test("A period starts where it says it does")
func aPeriodStartsWhereItSaysItDoes() {
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = TimeZone(identifier: "UTC")!
    let visit = now.addingTimeInterval(-5 * 3600)
    #expect(BoardSummary.start(of: .sinceLastVisit, lastVisit: visit, now: now, calendar: cal) == visit)
    #expect(BoardSummary.start(of: .sinceLastVisit, lastVisit: nil, now: now, calendar: cal) == now.addingTimeInterval(-day))
    #expect(BoardSummary.start(of: .lastHour, lastVisit: visit, now: now, calendar: cal) == now.addingTimeInterval(-3600))
    // 1_800_000_000 is 2027-01-15 08:00 UTC; midnight UTC is 1_799_971_200.
    #expect(BoardSummary.start(of: .today, lastVisit: visit, now: now, calendar: cal).timeIntervalSince1970 == 1_799_971_200)
    #expect(BoardSummary.Period.allCases.map(\.title) == ["Since Last Visit", "Last Hour", "Today"])
}

@Test("Notes are read only for tasks that moved since")
func notesAreReadOnlyForTasksThatMovedSince() {
    let since = now.addingTimeInterval(-3600)
    let rows = (0..<15).map { row("t\($0)", .inProgress, since: 100 + Double($0)) }
        + [row("still", .inProgress, since: 9000), row("x", .cancelled, since: 10)]
    let picked = BoardSummary.noteCandidates(rows: rows, since: since)
    #expect(picked.count == 10)
    #expect(picked.first?.key == "t0")
    #expect(!picked.contains { $0.key == "still" || $0.key == "x" })
}

@Test("A visit is kept per runner and workspace")
func aVisitIsKeptPerRunnerAndWorkspace() {
    let defaults = UserDefaults(suiteName: "ov80-\(UUID().uuidString)")!
    #expect(BoardVisit.read(host: "h", workspace: "w", from: defaults) == nil)
    BoardVisit.write(now, host: "h", workspace: "w", in: defaults)
    #expect(BoardVisit.read(host: "h", workspace: "w", from: defaults) == now)
    #expect(BoardVisit.read(host: "h", workspace: "other", from: defaults) == nil)
}

@Test("Today starts at local midnight on a spring-forward day")
func todayStartsAtLocalMidnightOnASpringForwardDay() {
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = TimeZone(identifier: "America/New_York")!
    // 2027-03-14 15:00 EDT (19:00 UTC), after the 02:00 jump. Midnight that day was EST.
    let at = Date(timeIntervalSince1970: 1_805_050_800)  // 2027-03-14 19:00 UTC
    let start = BoardSummary.start(of: .today, lastVisit: nil, now: at, calendar: cal)
    #expect(start.timeIntervalSince1970 == 1_805_000_400)  // 2027-03-14 05:00 UTC = 00:00 EST
    #expect(at.timeIntervalSince(start) == 14 * 3600, "only 14 hours elapsed, not 15: an hour was skipped")
}

@Test("A group is cut to five lines and says how many it left out")
func aGroupIsCutToFiveLines() {
    let items = (0..<8).map { BoardSummary.Item(id: "\($0)", taskID: "t", key: "k", title: "T", at: now) }
    let c = BoardSummary.capped(items)
    #expect(c.shown.count == 5)
    #expect(c.more == 3)
    #expect(BoardSummary.capped(Array(items.prefix(5))).more == 0)
}
