import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The board's "since you were last here" state (ov-80): a visit's baseline
/// holds still while the person reads, and the records read for the summary
/// are the few of tasks that moved, read once until they move again.
@MainActor
struct BoardSummaryStoreTests {
    private static let ws = "0198f2c0-0000-7000-8000-00000000c001"

    private func defaults() -> UserDefaults { UserDefaults(suiteName: "ov80-\(UUID().uuidString)")! }

    private func store(shows: @escaping @MainActor (String) -> Void = { _ in }) -> TaskBoardStore {
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
        return TaskBoardStore(client: client, workspace: .implicit(repository: Self.ws))
    }

    @Test func theBaselineHoldsStillUntilTheNextVisitBegins() {
        let defaults = defaults()
        let store = store()
        let long = Date(timeIntervalSince1970: 1_000_000)
        BoardVisit.write(long, host: store.hostKey, workspace: store.workspace.id, in: defaults)
        store.beginVisit(in: defaults)
        #expect(store.visitBaseline == long)
        // Leaving writes the new moment, and the summary on screen is unmoved.
        store.markVisited(in: defaults, now: long.addingTimeInterval(500))
        #expect(store.visitBaseline == long)
        store.beginVisit(in: defaults)
        #expect(store.visitBaseline == long.addingTimeInterval(500))
    }

    @Test func aMovedTasksDecisionIsReadOnceAndKept() async {
        var shown: [String] = []
        let store = store { shown.append($0) }
        await store.reload()
        let since = Date().addingTimeInterval(-3600)
        await store.readSummaryNotes(since: since)
        await store.readSummaryNotes(since: since)
        #expect(shown == ["a-1"], "read again though nothing moved")
        let summary = BoardSummary.make(rows: store.board.rows, notes: store.summaryNotes, since: since)
        #expect(summary.notes.map(\.detail) == ["Decision: Use SQLite"])
    }
}
