import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The navigator's lists after ov-103 and ov-104: compact rows cut by the
/// Done rule, the filter, the History page's place, header counts, and the
/// identities the lists animate by.
@MainActor
struct NavigatorListTests {
    private static let now = Date()
    private static let hour: TimeInterval = 3600

    private static func row(_ key: String, _ status: TaskStatus, ago: TimeInterval, title: String? = nil) -> TaskRow {
        let at = now.addingTimeInterval(-ago)
        return TaskRow(
            id: "id-\(key)", key: key, title: title ?? "Mac: task \(key)", status: status, statusSince: at,
            createdAt: at, updatedAt: at)
    }

    private static var sources: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/FarCooler")
    }

    /// ↑ and ↓ walk what's drawn: Done by its rule (unread, today, the floor
    /// of three), a long section's first ten, every match while filtering.
    @Test("The walk follows the Done rule, Show More and the filter")
    func theWalkFollowsTheCut() {
        let old = (0..<6).map { Self.row("d\($0)", .done, ago: 40 * 24 * Self.hour + Double($0)) }
        let todo = (0..<12).map { Self.row("t\($0)", .todo, ago: Self.hour, title: $0 == 11 ? "Phones: last" : nil) }
        let board = TaskBoardModel(columns: [
            TaskBoardColumn(status: .todo, rows: todo), TaskBoardColumn(status: .done, rows: old),
        ])
        let reads = BoardReads(floor: Self.now.addingTimeInterval(-24 * Self.hour))
        let walked = BoardKeys.rows(board, collapsed: [], reads: reads, now: Self.now)
        #expect(walked == todo.prefix(10).map(\.id) + ["id-d0", "id-d1", "id-d2"])
        #expect(BoardKeys.rows(board, collapsed: [], reads: reads, showingMore: [.todo], now: Self.now).count == 15)
        // Filtered: only the matches, every one of them, collapsed or not.
        let phones = BoardFilter.narrowed(board, "phones")
        #expect(BoardKeys.rows(phones, collapsed: [.todo], reads: reads, filtering: true, now: Self.now) == ["id-t11"])
        let all = BoardFilter.narrowed(board, "task d")
        #expect(BoardKeys.rows(all, collapsed: [], reads: reads, filtering: true, now: Self.now).count == 6)
    }

    /// Opening a finished row reads it, and it stays put while it's
    /// selected, so ↑ and ↓ go on from it (ov-104 review M2).
    @Test("An opened row stays in the walk until the selection moves on")
    func anOpenedRowStaysWhileSelected() {
        let today = (0..<3).map { Self.row("t\($0)", .done, ago: Double($0 + 1) * 60) }
        // Finished before today began, so only being unread keeps it.
        let start = Calendar.current.startOfDay(for: Self.now)
        let yesterday = TaskRow(
            id: "id-y", key: "y", title: "Mac: y", status: .done, statusSince: start.addingTimeInterval(-3600),
            updatedAt: start.addingTimeInterval(-3600))
        let board = TaskBoardModel(columns: [TaskBoardColumn(status: .done, rows: today + [yesterday])])
        var reads = BoardReads(floor: start.addingTimeInterval(-2 * 86_400))
        #expect(BoardKeys.rows(board, collapsed: [], reads: reads, now: Self.now).last == "id-y")
        reads.open(yesterday, now: Self.now)
        let walk = BoardKeys.rows(board, collapsed: [], reads: reads, keeping: "id-y", now: Self.now)
        #expect(walk.last == "id-y")
        #expect(BoardKeys.step(from: "id-y", by: -1, in: walk) == "id-t2")
        #expect(!BoardKeys.rows(board, collapsed: [], reads: reads, now: Self.now).contains("id-y"))
    }

    /// History's note hits count only for the query they answered.
    @Test("Stale note hits don't match the next query")
    func staleNoteHits() {
        let hits = NoteHits(query: "sqlite", ids: ["a"])
        #expect(hits.ids(for: "sqlite") == ["a"])
        #expect(hits.ids(for: "sqlit").isEmpty)
    }

    /// ⌘F says what it does, and Mark All as Read is offered beside a
    /// navigator only.
    @Test("⌘F's title and Mark All as Read follow the navigator")
    func theMenuFollowsTheNavigator() {
        let board = MainWindowFocus(overlayOpen: false, hasNavigator: true)
        let none = MainWindowFocus(overlayOpen: false)
        #expect(MainWindowFocus.findTitle(board) == "Filter Tasks")
        #expect(MainWindowFocus.findTitle(none) == "Find Workspace, Task, or Agent…")
        #expect(MainWindowFocus.marksRead(board))
        #expect(!MainWindowFocus.marksRead(none))
        #expect(!MainWindowFocus.marksRead(MainWindowFocus(overlayOpen: true, hasNavigator: true)))
    }

    /// The History page is a place the window remembers, and its breadcrumb
    /// names it.
    @Test("The History page is kept and named like any other place")
    func theHistoryPageIsAPlace() {
        let history = ContentView.Selection.workspace(host: "h", workspace: "w", focus: .history(.done))
        #expect(SelectionMemory.encode(history) == "h|w|history:done")
        #expect(SelectionMemory.encode(history).flatMap(SelectionMemory.decode) == history)
        #expect(SelectionMemory.decode("h|w|history:nonsense") == nil)
        let crumbs = WorkspaceNavigation.crumbs(history, trail: nil, workspace: "Overnight", task: { $0 }, worktree: { $0 })
        #expect(crumbs.map(\.title) == ["Overnight", "Done"])
        #expect(ContentView.healed(history, in: []) == history)
    }

    /// Every count beside a header is drawn by `SectionCount`, trailing,
    /// never "Finished (14)" (owner, ov-104).
    @Test("No navigator file puts a count in parentheses")
    func noParenthesizedCounts() throws {
        // Any interpolation alone in parentheses: "(\(n))", "(\(items.count))".
        let pattern = try Regex(#"\(\\\([^()]*\)\)"#)
        for file in ["TaskBoard.swift", "TaskListSection.swift", "BoardSummaryStrip.swift", "BoardHistoryView.swift", "Navigator.swift", "BoardWorktreesSection.swift"] {
            var text = try String(contentsOf: Self.sources.appendingPathComponent(file), encoding: .utf8)
            if let cut = text.range(of: "struct TaskCard: View") { text = String(text[..<cut.lowerBound]) }
            for (index, line) in text.split(separator: "\n", omittingEmptySubsequences: false).enumerated()
            where line.contains(pattern) && !line.contains("// not a count") {
                Issue.record("\(file):\(index + 1) parenthesizes a count: \(line.trimmingCharacters(in: .whitespaces))")
            }
        }
        // The scan sees them.
        #expect("Text(\"\\(title) (\\(items.count))\")".contains(pattern))
        #expect("Text(\"Finished (\\(n))\")".contains(pattern))
    }

    /// What the Unread list animates by: a ticket and what happened to it,
    /// a note by its own id, so a newer note on the same ticket arrives as
    /// new while its entry keeps its place.
    @Test("Unread's identities are the ticket's and the note's")
    func unreadIdentities() {
        let a = Self.row("a", .done, ago: Self.hour)
        let note = { (id: String, ago: TimeInterval) in
            TaskNoteRow(id: id, kind: .comment, actor: "user", at: Self.now.addingTimeInterval(-ago), body: "x")
        }
        let reads = BoardReads(floor: Self.now.addingTimeInterval(-24 * Self.hour))
        let one = BoardSummary.make(rows: [a], notes: ["id-a": [note("n1", 60)]], reads: reads)
        let two = BoardSummary.make(rows: [a], notes: ["id-a": [note("n1", 60), note("n2", 30)]], reads: reads)
        #expect(BoardSummaryStrip.identities(one) == ["id-a/done", "id-a/activity/n1"])
        #expect(BoardArrivals.new(old: BoardSummaryStrip.identities(one), now: BoardSummaryStrip.identities(two)) == ["id-a/activity/n2"])
    }

    /// The navigator's filter narrows Unread as it does the sections.
    @Test("The filter narrows Unread too")
    func theFilterNarrowsUnread() async {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            let at = Int64(Date().timeIntervalSince1970 * 1000) - 60_000
            let tasks = [("t1", "ov-1", "Mac: one"), ("t2", "ov-2", "Phones: two")].map { id, key, title in
                #"{"id":"\#(id)","key":"\#(key)","title":"\#(title)","status":"todo","status_since":\#(at),"created_at":\#(at),"updated_at":\#(at)}"#
            }
            return (Data(#"{"tasks":[\#(tasks.joined(separator: ","))]}"#.utf8), nil)
        }
        let store = TaskBoardStore(
            client: client, workspace: .implicit(repository: "r"),
            readStore: DefaultsBoardReads(UserDefaults(suiteName: "ov104-\(UUID().uuidString)")!))
        await store.reload()
        #expect(BoardSummaryStrip.summary(store: store, reads: store.reads, filter: "").created.count == 2)
        #expect(BoardSummaryStrip.summary(store: store, reads: store.reads, filter: "phones").created.map(\.key) == ["ov-2"])
    }

    /// The second line says what the header doesn't: never the status word.
    @Test("A row's second line leads with what the header doesn't say")
    func theSecondLine() {
        let working = TaskRowMeta.Agent(word: "claude working")
        let row = Self.row("w", .inProgress, ago: 60)
        #expect(TaskRowMeta.line(row, agent: working, at: Self.now).text == "claude working")
        #expect(TaskRowMeta.line(row, at: Self.now).text == "Added 1m ago")
        let asked = TaskRowMeta.line(row, agent: .init(word: "codex needs you", needsYou: true), at: Self.now)
        #expect(asked.tone == .attention)
        #expect(TaskRowMeta.line(Self.row("n", .needsDecision, ago: 60), at: Self.now).lead == "Answer to unblock")
        #expect(TaskRowMeta.word(.blocked) == "needs you")
        // Too narrow, the agent's word goes before the lead does.
        let long = TaskRowMeta.Line(lead: "Waiting on ov-90", agent: working, progress: "1 of 4", tone: .attention)
        #expect(TaskRowMetaView.without(long).agent == nil)
        #expect(TaskRowMetaView.without(.init(lead: nil, agent: working, progress: nil, tone: .quiet)).agent == working)
    }
}
