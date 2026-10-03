import Foundation
import Testing

@testable import AgentKit

// Unread (ov-104), Activity, History and the filter (ov-103).

private let now = Date(timeIntervalSince1970: 1_800_000_000)  // 2027-01-15 08:00 UTC, a Friday
private let hour: TimeInterval = 3600
private let day: TimeInterval = 24 * hour

private let utc: Calendar = {
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = TimeZone(identifier: "UTC")!
    cal.firstWeekday = 2
    return cal
}()

private func row(
    _ key: String, _ status: TaskStatus, since ago: TimeInterval, created: TimeInterval? = nil, title: String? = nil
) -> TaskRow {
    TaskRow(
        id: "id-\(key)", key: key, title: title ?? "Title \(key)", status: status,
        statusSince: now.addingTimeInterval(-ago), createdAt: created.map { now.addingTimeInterval(-$0) },
        updatedAt: now.addingTimeInterval(-ago))
}

private func note(_ id: String, _ kind: TaskNoteKind, ago: TimeInterval, _ body: String = "body") -> TaskNoteRow {
    TaskNoteRow(id: id, kind: kind, actor: "manager", at: now.addingTimeInterval(-ago), body: body)
}

@Test("Opening a ticket clears its unread items, and only its own")
func openingATicketClearsItsUnreadItems() {
    var reads = BoardReads(floor: now.addingTimeInterval(-day))
    let fin = row("fin", .done, since: hour)
    let new = row("new", .todo, since: 2 * hour, created: 2 * hour)
    let notes = ["id-new": [note("n1", .comment, ago: 30 * 60)]]
    let before = BoardSummary.make(rows: [fin, new], notes: notes, window: .unread(reads))
    #expect(before.finished.map(\.key) == ["fin"])
    #expect(before.created.map(\.key) == ["new"])
    #expect(before.activity.map(\.key) == ["new"])

    reads.open(new, now: now)
    let after = BoardSummary.make(rows: [fin, new], notes: notes, window: .unread(reads))
    #expect(after.created.isEmpty)
    #expect(after.activity.isEmpty)
    #expect(after.finished.map(\.key) == ["fin"], "another ticket's item stays")

    // A note written after it was opened is unread again.
    let later = ["id-new": [note("n2", .finding, ago: -60)]]
    #expect(BoardSummary.make(rows: [new], notes: later, window: .unread(reads)).activity.map(\.noteID) == ["n2"])

    // A runner clock ahead of this one: what it stamped is read on opening.
    var ahead = BoardReads(floor: now.addingTimeInterval(-day))
    let future = row("f", .done, since: -10 * 60)
    ahead.open(future, now: now)
    #expect(!ahead.finishedUnread(future))

    // Last Hour and Today don't read: opening changes nothing in them.
    #expect(BoardSummary.make(rows: [fin, new], notes: notes, window: .since(now.addingTimeInterval(-3 * hour))).created.map(\.key) == ["new"])

    // Mark All as Read.
    reads.markAllRead(rows: [fin, new], now: now)
    #expect(BoardSummary.make(rows: [fin, new], notes: notes, window: .unread(reads)).isEmpty)
    #expect(reads.opened.isEmpty)
}

@Test("Read state is kept per runner and workspace, and starts at the last visit")
func readStateIsKeptPerRunnerAndWorkspace() {
    let defaults = UserDefaults(suiteName: "ov104-\(UUID().uuidString)")!
    let store = DefaultsBoardReads(defaults)
    // Never seen: the last day.
    #expect(store.load(host: "h", workspace: "w", now: now).floor == now.addingTimeInterval(-day))
    // Since Last Visit's stamp, where there is one.
    BoardVisit.write(now.addingTimeInterval(-5 * hour), host: "h", workspace: "v", in: defaults)
    #expect(store.load(host: "h", workspace: "v", now: now).floor == now.addingTimeInterval(-5 * hour))

    var reads = store.load(host: "h", workspace: "w", now: now)
    reads.open(row("a", .todo, since: hour), now: now)
    store.save(reads, host: "h", workspace: "w")
    // Loaded later, it's the same, not a new first look.
    #expect(store.load(host: "h", workspace: "w", now: now.addingTimeInterval(3 * day)) == reads)
    #expect(store.load(host: "h", workspace: "other", now: now).opened.isEmpty)
    #expect(defaults.dictionary(forKey: "board.read.h.w.opened")?.keys.sorted() == ["id-a"])
}

@Test("Activity: one entry per ticket, its newest note whole, +N for the older")
func activityIsOneEntryPerTicket() {
    let rows = [row("a", .inProgress, since: hour), row("b", .inProgress, since: 2 * hour), row("x", .cancelled, since: hour)]
    let notes = [
        "id-a": [
            note("a1", .decision, ago: 50 * 60, "Use SQLite"),
            note("a2", .comment, ago: 10 * 60, "Looks\n\ngood   to me"),
            note("a3", .progress, ago: 30 * 60),
            note("a4", .statusChange, ago: 5 * 60),
            note("old", .finding, ago: 3 * day),
        ],
        "id-b": [note("b1", .question, ago: 20 * 60, "Which?")],
        "id-x": [note("x1", .finding, ago: 60)],
    ]
    let activity = BoardSummary.make(rows: rows, notes: notes, window: .since(now.addingTimeInterval(-day))).activity
    #expect(activity.map(\.key) == ["a", "b"])
    #expect(activity[0].noteID == "a2")
    #expect(activity[0].kind == .comment)
    #expect(activity[0].text == "Looks good to me")
    #expect(activity[0].more == 2)
    #expect(activity[0].moreLine == "+2 more")
    #expect(activity[1].more == 0)
    #expect(activity[1].moreLine == nil)
}

@Test("Identity is the ticket's, and the note's: what a list animates by")
func identityIsTheTicketsAndTheNotes() {
    let a = row("a", .done, since: hour)
    let notes = ["id-a": [note("n1", .comment, ago: 60)]]
    let one = BoardSummary.make(rows: [a], notes: notes, window: .since(now.addingTimeInterval(-day)))
    #expect(one.finished.map(\.id) == ["id-a/done"])
    #expect(one.activity.map(\.id) == ["id-a/activity"])
    // A newer note keeps the entry, under a new note id.
    let two = BoardSummary.make(
        rows: [a], notes: ["id-a": notes["id-a"]! + [note("n2", .decision, ago: 30)]],
        window: .since(now.addingTimeInterval(-day)))
    #expect(two.activity.map(\.id) == one.activity.map(\.id))
    #expect(two.activity.first?.noteID == "n2")
    // What arrived: by id, nothing on a first draw.
    #expect(BoardArrivals.new(old: nil, now: ["x"]).isEmpty)
    #expect(BoardArrivals.new(old: ["x", "y"], now: ["y", "z", "x"]) == ["z"])
}

@Test("History groups Today, Yesterday, This Week and Earlier, newest first")
func historyGroupsByWhenItLanded() {
    let rows = [
        row("old", .done, since: 40 * day),
        row("tue", .done, since: 3 * day),  // Tuesday 08:00, this week
        row("yday", .done, since: day),
        row("now", .done, since: hour),
        row("now2", .done, since: 2 * hour),
    ]
    let groups = BoardHistory.groups(rows, now: now, calendar: utc)
    #expect(groups.map(\.period) == [.today, .yesterday, .thisWeek, .earlier])
    #expect(groups.map(\.period.title) == ["Today", "Yesterday", "This Week", "Earlier"])
    #expect(groups[0].rows.map(\.key) == ["now", "now2"])
    #expect(groups[2].rows.map(\.key) == ["tue"])
    #expect(BoardHistory.groups([rows[0]], now: now, calendar: utc).map(\.period) == [.earlier])
    let en = Locale(identifier: "en_US")
    // The formatter's own narrow space before AM.
    #expect(BoardHistory.landed(rows[3], now: now, calendar: utc, locale: en).replacingOccurrences(of: "\u{202F}", with: " ") == "7:00 AM")
    #expect(BoardHistory.landed(rows[0], now: now, calendar: utc, locale: en) == "Dec 6, 2026")
}

@Test("History searches key, title and notes, and filters by area")
func historySearchesAndFiltersByArea() {
    let rows = [
        row("ov-1", .done, since: hour, title: "Mac: diff viewer shows an untracked file"),
        row("ov-2", .done, since: hour, title: "Daemon: hooks merge keeps a command"),
        row("ov-3", .done, since: hour, title: "Mac: title bar has two items"),
        row("ov-4", .done, since: hour, title: "A title with no area"),
    ]
    #expect(BoardHistory.area(of: "Mac: diff viewer") == "Mac")
    #expect(BoardHistory.area(of: "A title with no area") == nil)
    #expect(BoardHistory.area(of: "Why? Because: reasons") == nil)
    #expect(BoardHistory.areas(rows) == ["Mac", "Daemon"])
    #expect(BoardHistory.filter(rows, query: "").count == 4)
    #expect(BoardHistory.filter(rows, query: "", area: "Mac").map(\.key) == ["ov-1", "ov-3"])
    #expect(BoardHistory.filter(rows, query: "TITLE bar").map(\.key) == ["ov-3"])
    #expect(BoardHistory.filter(rows, query: "ov-2").map(\.key) == ["ov-2"])
    // A note search's hits match too, but only for a search.
    #expect(BoardHistory.filter(rows, query: "sqlite", noteHits: ["id-ov-4"]).map(\.key) == ["ov-4"])
    #expect(BoardHistory.filter(rows, query: "sqlite", area: "Mac", noteHits: ["id-ov-4"]).isEmpty)
}

@Test("The filter narrows every section, by every word")
func theFilterNarrowsEverySection() {
    let board = TaskBoardModel(columns: [
        TaskBoardColumn(status: .inProgress, rows: [row("ov-1", .inProgress, since: hour, title: "Mac: Unread replaces Since Last Visit")]),
        TaskBoardColumn(status: .done, rows: [
            row("ov-2", .done, since: hour, title: "Mac: diff viewer"),
            row("ov-3", .done, since: hour, title: "Phones: Café list"),
        ]),
    ])
    let mac = BoardFilter.narrowed(board, "mac")
    #expect(mac.columns.map { $0.rows.map(\.key) } == [["ov-1"], ["ov-2"]])
    #expect(BoardFilter.narrowed(board, "unread mac").rows.map(\.key) == ["ov-1"])
    #expect(BoardFilter.narrowed(board, "cafe").rows.map(\.key) == ["ov-3"])
    #expect(BoardFilter.narrowed(board, "OV-3").rows.map(\.key) == ["ov-3"])
    #expect(BoardFilter.narrowed(board, "nothing").columns.count == 2)
    #expect(BoardFilter.narrowed(board, "  ") == board)
    let summary = BoardSummary.make(rows: board.rows, window: .since(now.addingTimeInterval(-day)))
    #expect(summary.filtered { id in mac.rows.contains { $0.id == id } }.finished.map(\.key) == ["ov-2"])
}

@Test("Counts are said beside a title, never in parentheses")
func countsAreNeverParenthesized() {
    for line in [
        BoardDone.historyTitle(.done), BoardSectionCut.showMoreTitle(4),
        BoardSummary.collapsedLine(count: 3, period: .unread), BoardSummary.collapsedLine(count: 2, period: .today),
    ] {
        #expect(!line.contains("("), "\(line)")
    }
    #expect(BoardSummary.collapsedLine(count: 3, period: .unread) == "3 unread")
}
