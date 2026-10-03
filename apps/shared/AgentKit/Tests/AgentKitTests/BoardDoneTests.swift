import Foundation
import Testing

@testable import AgentKit

// Done shows what was finished lately, newest first (ov-80), and the summary
// says what changed since a moment.

private let now = Date(timeIntervalSince1970: 1_800_000_000)
private let day: TimeInterval = 24 * 60 * 60


/// Unread as of `start`: what a summary "since" a moment is, with nothing
/// opened.
private func unreadSince(_ start: Date) -> BoardReads { BoardReads(floor: start.addingTimeInterval(-0.001)) }

private func row(
    _ key: String, _ status: TaskStatus, since ago: TimeInterval, created: TimeInterval? = nil
) -> TaskRow {
    TaskRow(
        id: "id-\(key)", key: key, title: "Title \(key)", status: status,
        statusSince: now.addingTimeInterval(-ago),
        createdAt: created.map { now.addingTimeInterval(-$0) })
}

private let utc: Calendar = {
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = TimeZone(identifier: "UTC")!
    return cal
}()

// 1_800_000_000 is 2027-01-15 08:00 UTC, a Friday.

@Test("Done shows the unread and today's, newest first, with a floor of three")
func doneShowsUnreadAndTodaysWithAFloorOfThree() {
    // Read up to two days ago; three finished today (08:00 UTC), two
    // unread from yesterday, ten read last month.
    let reads = BoardReads(floor: now.addingTimeInterval(-2 * day))
    let today = (0..<3).map { row("t\($0)", .done, since: Double($0 + 1) * 600) }
    let unread = (0..<2).map { row("u\($0)", .done, since: 20 * 3600 + Double($0)) }
    let old = (0..<10).map { row("o\($0)", .done, since: 30 * day + Double($0)) }
    let shown = BoardDone.shown((old + unread + today).reversed(), reads: reads, now: now, calendar: utc)
    #expect(shown.map(\.key) == ["t0", "t1", "t2", "u0", "u1"])

    // Yesterday's, once opened, goes: it's read and not today's.
    var opened = reads
    opened.open(unread[0], now: now)
    #expect(BoardDone.shown(today + unread + old, reads: opened, now: now, calendar: utc).map(\.key) == ["t0", "t1", "t2", "u1"])

    // A quiet board: the newest three, however old.
    #expect(BoardDone.shown(old, reads: reads, now: now, calendar: utc).map(\.key) == ["o0", "o1", "o2"])
    #expect(BoardDone.shown(Array(old.prefix(2)), reads: reads, now: now, calendar: utc).count == 2)
    #expect(BoardDone.historyTitle(.done) == "All Done")
    #expect(BoardDone.historyTitle(.cancelled) == "All Canceled")
}

@Test("Canceled works the same, with the History row; other sections cut at ten")
func canceledWorksTheSameAndOthersCutAtTen() {
    let reads = BoardReads(floor: now.addingTimeInterval(-2 * day))
    let canceled = TaskBoardColumn(status: .cancelled, rows: (0..<15).map { row("c\($0)", .cancelled, since: 40 * day + Double($0)) })
    let c = canceled.cut(reads: reads, now: now, calendar: utc)
    #expect(c.rows.map(\.key) == ["c0", "c1", "c2"])
    #expect(c.history == 15)
    // Every one drawn: no History row (ov-104 review).
    let few = TaskBoardColumn(status: .cancelled, rows: [row("c", .cancelled, since: 40 * day)])
    #expect(few.cut(reads: reads, now: now, calendar: utc).history == nil)
    #expect(c.hidden == 0)
    // Filtering shows every match.
    #expect(canceled.cut(reads: reads, filtering: true, now: now, calendar: utc).rows.count == 15)

    let todo = TaskBoardColumn(status: .todo, rows: (0..<14).map { row("d\($0)", .todo, since: 40 * day) })
    let t = todo.cut(reads: reads, now: now, calendar: utc)
    #expect(t.rows.count == 10)
    #expect(t.hidden == 4)
    #expect(t.history == nil)
    #expect(BoardSectionCut.showMoreTitle(t.hidden) == "Show 4 More")
    #expect(todo.cut(reads: reads, showingAll: true, now: now, calendar: utc).rows.count == 14)
    #expect(TaskBoardColumn(status: .done, rows: []).cut(reads: reads, now: now).history == nil)
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
            TaskNoteRow(id: "n2", kind: .comment, actor: "user", at: now.addingTimeInterval(-120), body: "meh"),
            TaskNoteRow(id: "n3", kind: .finding, actor: "user", at: now.addingTimeInterval(-9000), body: "old"),
        ]
    ]
    let summary = BoardSummary.make(rows: rows, notes: notes, reads: unreadSince(since))
    #expect(summary.finished.map(\.key) == ["fin"])
    #expect(summary.moved.map(\.key) == ["rev", "nd"])
    #expect(summary.moved.first?.detail == "In Review")
    #expect(summary.created.map(\.key) == ["new"])
    // Activity: every kind a person or agent writes, one entry per ticket.
    #expect(summary.activity.map(\.key) == ["oldnew"])
    #expect(summary.activity.first?.kind == .decision)
    #expect(summary.activity.first?.text == "Use SQLite because")
    #expect(summary.activity.first?.more == 1)
    #expect(!summary.isEmpty)
    #expect(BoardSummary.make(rows: rows, reads: unreadSince(now)).isEmpty)
    #expect(BoardSummary.nothing == "You’re all caught up.")
}

@Test("Done's today turns over at midnight")
func donesTodayTurnsOverAtMidnight() {
    // Read through everything: only the floor and today keep a row.
    let reads = BoardReads(floor: now)
    // 23:59 and 00:01 UTC on the 14th/15th, and three older ones.
    let late = TaskRow(id: "late", key: "late", title: "", status: .done, statusSince: Date(timeIntervalSince1970: 1_799_971_140))
    let early = TaskRow(id: "early", key: "early", title: "", status: .done, statusSince: Date(timeIntervalSince1970: 1_799_971_260))
    let old = (0..<3).map { row("o\($0)", .done, since: 30 * day + Double($0)) }
    let before = Date(timeIntervalSince1970: 1_799_971_150)  // 23:59:50 on the 14th
    let after = Date(timeIntervalSince1970: 1_799_971_300)  // 00:01:40 on the 15th
    #expect(BoardDone.shown([late] + old, reads: reads, now: before, calendar: utc).map(\.key) == ["late", "o0", "o1"])
    // Past midnight, yesterday's late finish is no longer today's: only the floor of 3 holds.
    #expect(BoardDone.shown([early, late] + old, reads: reads, now: after, calendar: utc).map(\.key) == ["early", "late", "o0"])
    let more = (0..<3).map { TaskRow(id: "e\($0)", key: "e\($0)", title: "", status: .done, statusSince: Date(timeIntervalSince1970: 1_799_971_270 + Double($0))) }
    #expect(!BoardDone.shown(more + [late], reads: reads, now: after, calendar: utc).contains { $0.key == "late" })
}

@Test("The task selected stays in its section after opening reads it")
func theSelectedTaskStaysAfterOpening() {
    var reads = BoardReads(floor: now.addingTimeInterval(-2 * day))
    let today = (0..<3).map { row("t\($0)", .done, since: Double($0 + 1) * 600) }
    let yesterday = row("y", .done, since: 20 * 3600)
    let done = TaskBoardColumn(status: .done, rows: today + [yesterday])
    #expect(done.cut(reads: reads, now: now, calendar: utc).rows.map(\.key) == ["t0", "t1", "t2", "y"])
    reads.open(yesterday, now: now)
    // Read, and selected: kept.
    #expect(done.cut(reads: reads, keeping: "id-y", now: now, calendar: utc).rows.map(\.key) == ["t0", "t1", "t2", "y"])
    // The selection moved on: it goes to History.
    #expect(done.cut(reads: reads, now: now, calendar: utc).rows.map(\.key) == ["t0", "t1", "t2"])
    // A long section keeps it past ten too.
    let todo = TaskBoardColumn(status: .todo, rows: (0..<12).map { row("d\($0)", .todo, since: day) })
    let kept = todo.cut(reads: reads, keeping: "id-d11", now: now, calendar: utc)
    #expect(kept.rows.last?.key == "d11")
    #expect(kept.hidden == 1)
}

@Test("Notes are read only for tasks that moved since")
func notesAreReadOnlyForTasksThatMovedSince() {
    let since = now.addingTimeInterval(-3600)
    let rows = (0..<15).map { row("t\($0)", .inProgress, since: 100 + Double($0)) }
        + [row("still", .inProgress, since: 9000), row("x", .cancelled, since: 10)]
    let picked = BoardSummary.noteCandidates(rows: rows, reads: unreadSince(since), limit: 10)
    #expect(picked.count == 10)
    #expect(BoardSummary.noteCandidates(rows: rows, reads: unreadSince(since)).count == 15)
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

@Test("A group is cut to five lines and says how many it left out")
func aGroupIsCutToFiveLines() {
    let items = (0..<8).map { BoardSummary.Item(id: "\($0)", taskID: "t", key: "k", title: "T", at: now) }
    let c = BoardSummary.capped(items)
    #expect(c.shown.count == 5)
    #expect(c.more == 3)
    #expect(BoardSummary.capped(Array(items.prefix(5))).more == 0)
}
