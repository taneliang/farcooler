import Foundation
import Testing
@testable import AgentKit

/// A runner's key index is built once per change in what it links, and is
/// the same value while that's the same (ov-791): a linker made on every
/// pass then compares with the last at once, not by walking every task.
@MainActor
@Suite struct TaskKeyIndexCacheTests {
    static let main = WorkspaceSummary(id: "w-main", name: "Main", taskPrefix: "ov", isMain: true, ordinal: 0)

    /// A board of `count` tasks, the first `moved` of them in progress.
    static func boards(_ count: Int, moved: Int = 0) -> [String: TaskBoardModel] {
        let rows = (0..<count).map { n in
            TaskRow(id: "t\(n)", key: "ov-\(n)", title: "Task \(n)", status: n < moved ? .inProgress : .todo, statusSince: .distantPast)
        }
        return ["w-main": TaskBoardModel(columns: [TaskBoardColumn(status: .todo, rows: rows)])]
    }

    @Test func aBoardReadAgainKeepsTheIndexWhileItsKeysAreTheSame() {
        let cache = TaskKeyIndexCache()
        let first = cache.index(runner: "R1", workspaces: [Self.main], boards: Self.boards(800))
        #expect(cache.builds == 1)
        // The same read: handed back as it is.
        _ = cache.index(runner: "R1", workspaces: [Self.main], boards: Self.boards(800))
        // A task moved, as a busy runner's do every second: a new board,
        // the same keys, so the same index.
        let moved = cache.index(runner: "R1", workspaces: [Self.main], boards: Self.boards(800, moved: 3))
        #expect(moved == first)
        #expect(cache.builds == 1, "a move that links nothing new built the index \(cache.builds) times")
        // A new task: a new index, which links it.
        let grown = cache.index(runner: "R1", workspaces: [Self.main], boards: Self.boards(801))
        #expect(cache.builds == 2)
        #expect(grown.targets["ov-800"] != nil)
        #expect(cache.index(runner: "R2", workspaces: [Self.main], boards: Self.boards(801)).runner == "R2")
    }

    /// The phone's linker takes its index from the cache it's given.
    @Test func thePhonesLinkerUsesTheCache() {
        let cache = TaskKeyIndexCache()
        for moved in 0..<5 {
            let linker = TaskKeyLinker.phone(
                runner: "R1", workspaces: [Self.main], boards: Self.boards(200, moved: moved), indexes: cache) { _ in }
            #expect(linker.index.targets.count == 200)
        }
        #expect(cache.builds == 1)
    }
}
