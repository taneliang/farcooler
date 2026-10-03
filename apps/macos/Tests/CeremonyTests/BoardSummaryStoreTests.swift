import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The board's Unread state (ov-104): opening a ticket reads it, and is kept
/// per runner and workspace on this Mac; the records read for the summary
/// are the few of tasks that moved, read once until they move again.
@MainActor
struct BoardSummaryStoreTests {
    private static let ws = "0198f2c0-0000-7000-8000-00000000c001"

    private func defaults() -> UserDefaults { UserDefaults(suiteName: "ov80-\(UUID().uuidString)")! }

    private func store(
        defaults: UserDefaults = UserDefaults(suiteName: "ov104-\(UUID().uuidString)")!,
        shows: @escaping @MainActor (String) -> Void = { _ in }
    ) -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            let words = args.filter { $0 != "--json" }
            if words.starts(with: ["task", "list"]) {
                let now = Int64(Date().timeIntervalSince1970 * 1000)
                return (Data(#"{"tasks":[{"id":"t1","key":"a-1","title":"Pick","status":"in_progress","status_since":\#(now),"updated_at":\#(now)}]}"#.utf8), nil)
            }
            if words.starts(with: ["task", "show"]) {
                await MainActor.run { shows(words[2]) }
                let now = Int64(Date().timeIntervalSince1970 * 1000)
                return (Data(#"{"notes":[{"id":"n1","kind":"decision","actor":"user","at":\#(now),"body":"Use SQLite"}],"blocks":[]}"#.utf8), nil)
            }
            return (Data(), nil)
        }
        return TaskBoardStore(
            client: client, workspace: .implicit(repository: Self.ws), readStore: DefaultsBoardReads(defaults))
    }

    /// Opening a ticket clears its Unread items, and a store made again,
    /// as the next launch does, finds it read.
    @Test func openingATicketReadsItAndItStaysRead() async throws {
        let defaults = defaults()
        let store = store(defaults: defaults)
        await store.reload()
        let window = BoardSummary.window(.unread, reads: store.reads, now: Date())
        await store.readSummaryNotes(window: window)
        let before = BoardSummary.make(rows: store.board.rows, notes: store.summaryNotes, window: window)
        #expect(before.activity.map(\.key) == ["a-1"])

        let row = try #require(store.board.rows.first)
        await store.open(row)
        let after = BoardSummary.make(
            rows: store.board.rows, notes: store.summaryNotes, window: .unread(store.reads))
        #expect(after.isEmpty, "still unread after opening: \(after)")

        let again = self.store(defaults: defaults)
        #expect(again.reads.opened.keys.sorted() == ["t1"])
        #expect(abs(again.reads.floor.timeIntervalSince(store.reads.floor)) < 0.001)
        #expect(!BoardSummary.make(rows: store.board.rows, notes: store.summaryNotes, window: .unread(again.reads)).activity.contains { $0.key == "a-1" })
    }

    /// Mark All as Read empties Unread, and Last Hour still lists the same.
    @Test func markAllAsReadEmptiesUnreadOnly() async {
        let store = store()
        await store.reload()
        let hour = BoardSummary.Window.since(Date().addingTimeInterval(-3600))
        await store.readSummaryNotes(window: hour)
        store.markAllRead()
        #expect(BoardSummary.make(rows: store.board.rows, notes: store.summaryNotes, window: .unread(store.reads)).isEmpty)
        #expect(!BoardSummary.make(rows: store.board.rows, notes: store.summaryNotes, window: hour).isEmpty)
    }

    @Test func aMovedTasksDecisionIsReadOnceAndKept() async {
        var shown: [String] = []
        let store = store { shown.append($0) }
        await store.reload()
        let window = BoardSummary.Window.since(Date().addingTimeInterval(-3600))
        await store.readSummaryNotes(window: window)
        await store.readSummaryNotes(window: window)
        #expect(shown == ["a-1"], "read again though nothing moved")
        let summary = BoardSummary.make(rows: store.board.rows, notes: store.summaryNotes, window: window)
        #expect(summary.activity.map(\.text) == ["Use SQLite"])
    }

    /// A strip expanded after launch reads its notes then: expanding changes the key.
    @Test func expandingTheStripChangesWhatTheNotesReadIsKeyedOn() {
        let at = Date(timeIntervalSince1970: 1_800_000_000)
        let closed = BoardSummaryStrip.notesKey(window: .since(at), generation: 1, count: 3, collapsed: true)
        let open = BoardSummaryStrip.notesKey(window: .since(at), generation: 1, count: 3, collapsed: false)
        #expect(closed != open)
        // A ticket opened reads the notes again.
        var reads = BoardReads(floor: at)
        let before = BoardSummaryStrip.notesKey(window: .unread(reads), generation: 1, count: 3, collapsed: false)
        reads.open(TaskRow(id: "t", key: "k", title: "", status: .todo, statusSince: at), now: at)
        #expect(BoardSummaryStrip.notesKey(window: .unread(reads), generation: 1, count: 3, collapsed: false) != before)
    }
}
