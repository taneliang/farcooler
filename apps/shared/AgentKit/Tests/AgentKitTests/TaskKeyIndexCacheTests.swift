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

    /// Anything the links read is a new index: a key gone, a task moved
    /// to another workspace under the same key, a prefix changed.
    @Test func whatTheLinksReadMakesANewIndex() {
        let cache = TaskKeyIndexCache()
        let before = cache.index(runner: "R1", workspaces: [Self.main], boards: Self.boards(10))
        let fewer = cache.index(runner: "R1", workspaces: [Self.main], boards: Self.boards(9))
        #expect(fewer.targets["ov-9"] == nil && before.targets["ov-9"] != nil)
        #expect(cache.builds == 2)

        var moved = Self.boards(9)
        let side = WorkspaceSummary(id: "w-side", name: "Side", taskPrefix: "ov", isMain: false, ordinal: 1)
        let row = TaskRow(id: "t3", key: "ov-3", title: "Task 3", status: .todo, statusSince: .distantPast)
        moved["w-side"] = TaskBoardModel(columns: [TaskBoardColumn(status: .todo, rows: [row])])
        moved["w-main"] = TaskBoardModel(columns: [
            TaskBoardColumn(status: .todo, rows: (moved["w-main"]?.rows ?? []).filter { $0.key != "ov-3" })
        ])
        let there = cache.index(runner: "R1", workspaces: [Self.main, side], boards: moved)
        #expect(there.targets["ov-3"]?.workspace == "w-side")
        #expect(cache.builds == 3)

        let renamed = WorkspaceSummary(id: "w-main", name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0)
        let prefixed = cache.index(runner: "R1", workspaces: [renamed, side], boards: moved)
        #expect(prefixed.prefixes == ["fc", "ov"])
        #expect(cache.builds == 4)
    }

    /// Given the boards' version, the cache compares it, not the boards:
    /// the same version is the last index, whatever is passed; a new one is
    /// read (ov-791).
    @Test func aVersionStandsForTheBoards() {
        let cache = TaskKeyIndexCache()
        _ = cache.index(runner: "R1", workspaces: [Self.main], boards: Self.boards(10), version: 1)
        let same = cache.index(runner: "R1", workspaces: [Self.main], boards: Self.boards(11), version: 1)
        #expect(same.targets["ov-10"] == nil, "the same version read the boards again")
        let next = cache.index(runner: "R1", workspaces: [Self.main], boards: Self.boards(11), version: 2)
        #expect(next.targets["ov-10"] != nil)
        #expect(cache.builds == 2)
    }

    /// The card cache too, its version standing for the boards and plans.
    @Test func aVersionStandsForTheCardsReads() {
        let cache = TaskKeyCardCache()
        _ = cache.cards(runner: "R1", boards: Self.boards(10), version: [1, 1])
        _ = cache.cards(runner: "R1", boards: Self.boards(10, moved: 2), version: [1, 1])
        #expect(cache.builds == 1)
        _ = cache.cards(runner: "R1", boards: Self.boards(10, moved: 2), version: [2, 1])
        _ = cache.cards(runner: "R1", boards: Self.boards(10, moved: 2), version: [2, 2])
        #expect(cache.builds == 3)
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
