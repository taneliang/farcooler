import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Read state kept on the runner (ov-113): Unread reads it when the runner has
/// it, sends marks and Mark All as Read to it, hears other devices through its
/// events, and uploads this Mac's own once.
@MainActor
struct BoardReadSyncTests {
    nonisolated private static let repo = "0198f2c0-0000-7000-8000-00000000c001"
    nonisolated private static let ws = "0198f2c0-0000-7000-8000-00000000c002"
    nonisolated private static let task = "0198f2c0-0000-7000-8000-00000000c003"
    nonisolated private static let moved = 1_700_000_000_000 as Int64

    /// A runner, as far as these tests ask of one: the board it lists, with
    /// `reads` when it keeps them, and every `board mark-read` it was sent.
    final class Runner {
        /// The state it keeps, as the real runner does: merged by max.
        var reads: String? {
            didSet { floor = reads.flatMap { WireBoardReads.decode(state: Data($0.utf8)) }.map(\.floorMs) ?? 0 }
        }
        private var floor: Int64 = 0
        private var marks: [String: Int64] = [:]
        var sent: [[String]] = []
        var failMarks = false

        /// `board mark-read`'s answer, after merging what it was told.
        func merge(_ words: [String]) -> Data {
            var i = 0
            while i < words.count {
                if words[i] == "--floor" { floor = max(floor, Int64(words[i + 1])!) }
                if words[i] == "--task" {
                    let pair = words[i + 1].split(separator: ":")
                    marks[String(pair[0]), default: 0] = max(marks[String(pair[0])] ?? 0, Int64(pair[1])!)
                }
                i += 1
            }
            return Data(state().utf8)
        }

        func state() -> String {
            BoardReadSyncTests.state(floor: floor, opened: marks.filter { $0.value > floor }.map { ($0.key, $0.value) })
        }

        func board() -> Data {
            let tail = reads == nil ? "" : #","reads":\#(state())"#
            return Data(
                #"{"tasks":[{"id":"\#(BoardReadSyncTests.task)","key":"a-1","title":"Pick","status":"done","status_since":\#(BoardReadSyncTests.moved),"updated_at":\#(BoardReadSyncTests.moved)}]\#(tail)}"#
                    .utf8)
        }
    }

    nonisolated static func state(floor: Int64, opened: [(String, Int64)] = []) -> String {
        let marks = opened.map { #"{"task_id":"\#($0.0)","opened_ms":\#($0.1)}"# }.joined(separator: ",")
        return #"{"workspace_id":"\#(ws)","floor_ms":\#(floor),"opened":[\#(marks)]}"#
    }

    private func store(_ runner: Runner, defaults: UserDefaults) -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            let words = args.filter { $0 != "--json" }
            if words.starts(with: ["task", "list"]) { return (runner.board(), nil) }
            if words.starts(with: ["task", "show"]) { return (Data(#"{"notes":[],"blocks":[]}"#.utf8), nil) }
            if words.starts(with: ["board", "mark-read"]) {
                runner.sent.append(words)
                if runner.failMarks { return (nil, "no") }
                return (runner.merge(words), nil)
            }
            return (Data(), nil)
        }
        let workspace = WorkspaceSummary(id: Self.ws, name: "Evals", taskPrefix: "ev", isMain: false, ordinal: 1, repository: Self.repo)
        return TaskBoardStore(client: client, workspace: workspace, readStore: DefaultsBoardReads(defaults))
    }

    /// A Mac that has long kept its own state, floor far back, so what it
    /// holds can't hide anything a test is about. (Moments are in 2023, long
    /// before this Mac's clock, which is what lets a mark made with the
    /// device clock be told from one made with the runner's.)
    private func defaults() -> UserDefaults {
        let defaults = UserDefaults(suiteName: "ov113-\(UUID().uuidString)")!
        DefaultsBoardReads(defaults).save(
            BoardReads(floor: Date(timeIntervalSince1970: 1)), host: "local", workspace: Self.ws)
        return defaults
    }

    private func unread(_ store: TaskBoardStore) -> Bool {
        store.board.rows.contains { store.reads.finishedUnread($0) }
    }

    /// The runner's state decides Unread, over this Mac's own.
    @Test func unreadReadsFromTheRunner() async {
        let runner = Runner()
        runner.reads = Self.state(floor: Self.moved + 1000)
        let store = store(runner, defaults: defaults())
        await store.reload()
        #expect(store.runnerKeepsReads)
        #expect(!unread(store), "the runner says it's read")
        #expect(store.reads.floor == Date(timeIntervalSince1970: Double(Self.moved + 1000) / 1000))
    }

    /// An old runner sends no state: Unread is this Mac's, and saved here.
    @Test func anOldRunnerKeepsTheLocalState() async throws {
        let runner = Runner()
        let defaults = defaults()
        let store = store(runner, defaults: defaults)
        await store.reload()
        #expect(!store.runnerKeepsReads)
        #expect(unread(store))
        await store.open(try #require(store.board.rows.first))
        #expect(runner.sent.isEmpty)
        #expect(!unread(store))
        #expect(defaults.dictionary(forKey: DefaultsBoardReads.openedKey(host: "local", workspace: Self.ws))?[Self.task] != nil)
    }

    /// Opening a ticket sends the runner's own time, and keeps nothing here.
    @Test func aMarkIsWrittenThroughTheRunner() async throws {
        let runner = Runner()
        runner.reads = Self.state(floor: Self.moved - 5000)
        let defaults = defaults()
        let store = store(runner, defaults: defaults)
        await store.reload()
        runner.sent = []
        #expect(unread(store))
        await store.open(try #require(store.board.rows.first))
        // Not `flushReads()`: the mark has to send itself.
        await store.flushChain?.value
        #expect(!unread(store))
        #expect(runner.sent == [["board", "mark-read", "--repo", Self.repo, "--workspace", Self.ws, "--task", "\(Self.task):\(Self.moved)"]])
        #expect(defaults.dictionary(forKey: DefaultsBoardReads.openedKey(host: "local", workspace: Self.ws))?[Self.task] == nil)
    }

    /// A send the runner never answered stays owed, and goes with the next one.
    @Test func aFailedSendIsSentAgain() async throws {
        let runner = Runner()
        runner.reads = Self.state(floor: Self.moved - 5000)
        let store = store(runner, defaults: defaults())
        await store.reload()
        runner.sent = []
        runner.failMarks = true
        await store.open(try #require(store.board.rows.first))
        await store.flushReads()
        #expect(!unread(store), "read here at once")
        runner.failMarks = false
        runner.sent = []
        await store.flushReads()
        #expect(runner.sent == [["board", "mark-read", "--repo", Self.repo, "--workspace", Self.ws, "--task", "\(Self.task):\(Self.moved)"]])
        runner.sent = []
        await store.flushReads()
        #expect(runner.sent.isEmpty, "once told, not told again")
    }

    /// Mark All as Read sends a floor, which clears Unread.
    @Test func markAllAsReadSendsAFloor() async throws {
        let runner = Runner()
        runner.reads = Self.state(floor: Self.moved - 5000)
        let store = store(runner, defaults: defaults())
        await store.reload()
        runner.sent = []
        store.askToMarkAllRead(.granting)
        await store.flushChain?.value
        #expect(!unread(store))
        #expect(runner.sent == [["board", "mark-read", "--repo", Self.repo, "--workspace", Self.ws, "--floor", "\(Self.moved)"]])
    }

    /// Another device read it: the runner's event clears the line.
    @Test func aReadsEventUpdatesUnread() async throws {
        let runner = Runner()
        runner.reads = Self.state(floor: Self.moved - 5000)
        let store = store(runner, defaults: defaults())
        await store.reload()
        #expect(unread(store))
        let heard = try #require(
            WireBoardReads.decode(state: Data(Self.state(floor: Self.moved - 5000, opened: [(Self.task, Self.moved)]).utf8)))
        store.client.readsChanged(heard)
        #expect(!unread(store))
        // A lower state, late, undoes nothing.
        store.client.readsChanged(try #require(WireBoardReads.decode(state: Data(Self.state(floor: 0).utf8))))
        #expect(!unread(store))
    }

    /// At the upgrade this Mac's floor and marks go up once, and not again.
    @Test func theLocalStateIsUploadedOnce() async throws {
        let runner = Runner()
        runner.reads = Self.state(floor: Self.moved - 9_000_000)
        let defaults = defaults()
        let local = DefaultsBoardReads(defaults)
        local.save(
            BoardReads(
                floor: Date(timeIntervalSince1970: Double(Self.moved - 5000) / 1000),
                opened: [Self.task: Date(timeIntervalSince1970: Double(Self.moved + 1000) / 1000)]),
            host: "local", workspace: Self.ws)
        let store = store(runner, defaults: defaults)
        await store.reload()
        #expect(runner.sent == [[
            "board", "mark-read", "--repo", Self.repo, "--workspace", Self.ws, "--task", "\(Self.task):\(Self.moved + 1000)",
            "--floor", "\(Self.moved - 5000)",
        ]])
        #expect(local.isUploaded(host: "local", workspace: Self.ws))
        runner.sent = []
        await store.reload()
        // The runner kept it, as the real one does; a fresh launch finds it done.
        let again = self.store(runner, defaults: defaults)
        await again.reload()
        #expect(runner.sent.isEmpty, "uploaded already")
    }
}
