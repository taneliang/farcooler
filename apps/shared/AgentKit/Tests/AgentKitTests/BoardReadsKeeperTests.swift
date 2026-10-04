import Foundation
import Testing

@testable import AgentKit

// A phone's board reads (ov-113): the runner's state, marks and Mark All as
// Read sent through it, the one upload, and the fallback to this phone's own.

@MainActor
struct BoardReadsKeeperTests {
    private static let task = "task-a"
    /// A moment in 2027, well past any `now` of this phone's that a test uses
    /// as "the past", so a device clock can be told from the runner's.
    private static let moved = 1_800_000_000_000 as Int64
    private static let host = "runner-1"
    private static let ws = "ws-1"

    private func row(_ id: String = BoardReadsKeeperTests.task, movedMs: Int64 = BoardReadsKeeperTests.moved) -> TaskRow {
        let at = Date(timeIntervalSince1970: Double(movedMs) / 1000)
        return TaskRow(id: id, key: "k-\(id)", title: id, status: .done, statusSince: at, createdAt: nil, updatedAt: at)
    }

    /// A runner, as far as these tests ask: keeps state by max, skips the
    /// tasks it's told to, and records every send.
    @MainActor final class Runner {
        var floor: Int64 = 0
        var marks: [String: Int64] = [:]
        var sent: [ReadsRaise] = []
        var fails = false
        var skips: Set<String> = []
        private var held: [CheckedContinuation<Void, Never>] = []
        var holds = false
        func release() { holds = false; held.forEach { $0.resume() }; held = [] }

        func answer(_ raise: ReadsRaise) async -> Data? {
            sent.append(raise)
            if holds { await withCheckedContinuation { held.append($0) } }
            if fails { return nil }
            if let f = raise.floor { floor = max(floor, BoardReads.milliseconds(f)) }
            for (id, at) in raise.opened where !skips.contains(id) {
                marks[id] = max(marks[id] ?? 0, BoardReads.milliseconds(at))
            }
            return Data(state().utf8)
        }

        func state() -> String {
            let kept = marks.filter { $0.value > floor }.map { #"{"task_id":"\#($0.key)","opened_ms":\#($0.value)}"# }
            return #"{"workspace_id":"ws-1","floor_ms":\#(floor),"opened":[\#(kept.joined(separator: ","))]}"#
        }

        func board() -> Data { Data(#"{"tasks":[],"reads":\#(state())}"#.utf8) }
    }

    private func defaults() -> DefaultsBoardReads {
        DefaultsBoardReads(UserDefaults(suiteName: "ov113-phone-\(UUID().uuidString)")!)
    }

    private func keeper(_ runner: Runner, _ store: DefaultsBoardReads, now: Date = Date()) -> BoardReadsKeeper {
        BoardReadsKeeper(store: store, host: Self.host, workspace: Self.ws, now: now) { await runner.answer($0) }
    }

    private func ms(_ value: Int64) -> Date { Date(timeIntervalSince1970: Double(value) / 1000) }

    @Test func theRunnersStateDecidesWhatIsRead() {
        let runner = Runner()
        runner.floor = Self.moved + 1000
        let k = keeper(runner, defaults())
        #expect(!k.runnerKeepsReads)
        k.adopt(board: runner.board())
        #expect(k.runnerKeepsReads)
        #expect(!k.reads.finishedUnread(row()), "the runner says it's read")
        #expect(k.reads.floor == ms(Self.moved + 1000))
    }

    @Test func aMarkIsSentWithTheRunnersTimeAndKeptNowhereElse() async {
        let runner = Runner()
        let store = defaults()
        let k = keeper(runner, store)
        k.adopt(board: runner.board())
        await k.flush()
        k.open(row(), now: Date(timeIntervalSince1970: 4_100_000_000))
        #expect(!k.reads.finishedUnread(row()), "read at once, before the runner answers")
        await k.flush()
        #expect(runner.sent.last?.opened[Self.task] == ms(Self.moved), "the row's own time, not this phone's clock")
        #expect(runner.marks[Self.task] == Self.moved)
        #expect(store.loadPending(host: Self.host, workspace: Self.ws).isEmpty, "settled once it answered")
        #expect(store.load(host: Self.host, workspace: Self.ws, now: Date()).opened[Self.task] == nil, "this phone keeps nothing")
    }

    @Test func markAllAsReadSendsAFloorThroughWhatWasShown() async {
        let runner = Runner()
        let k = keeper(runner, defaults())
        k.adopt(board: runner.board())
        k.markAllRead(rows: [row(), row("b", movedMs: Self.moved + 500)], latest: ms(Self.moved + 900))
        await k.flush()
        #expect(runner.floor == Self.moved + 900)
        #expect(k.reads.floor == ms(Self.moved + 900))
    }

    @Test func aStateHeardLateOrLowerNeverLowersAMark() {
        let runner = Runner()
        runner.floor = Self.moved + 5000
        let k = keeper(runner, defaults())
        k.adopt(board: runner.board())
        k.heard(WireBoardReads(workspaceID: Self.ws, floorMs: Self.moved, opened: []))
        #expect(k.reads.floor == ms(Self.moved + 5000))
        k.heard(WireBoardReads(workspaceID: Self.ws, floorMs: Self.moved + 9000))
        #expect(k.reads.floor == ms(Self.moved + 9000), "a higher one rises")
    }

    @Test func aMarkNotYetAnsweredSurvivesAStateHeardWithoutIt() {
        let runner = Runner()
        runner.fails = true
        let k = keeper(runner, defaults())
        k.adopt(board: runner.board())
        k.open(row())
        k.heard(WireBoardReads(workspaceID: Self.ws, floorMs: 1))
        #expect(!k.reads.finishedUnread(row()), "a state without the mark doesn't undo it")
    }

    @Test func openingNeverLowersAMarkHeardFromAnotherDevice() {
        let runner = Runner()
        runner.marks[Self.task] = Self.moved + 4000
        let k = keeper(runner, defaults())
        k.adopt(board: runner.board())
        k.open(row(), latest: nil)
        #expect(k.reads.opened[Self.task] == ms(Self.moved + 4000))
    }

    @Test func anUnsentMarkSurvivesARelaunchAndGoesOnTheNextRead() async {
        let runner = Runner()
        runner.fails = true
        let store = defaults()
        let first = keeper(runner, store)
        first.adopt(board: runner.board())
        first.open(row())
        await first.flush()
        #expect(!store.loadPending(host: Self.host, workspace: Self.ws).isEmpty, "no answer keeps it owed")

        runner.fails = false
        let second = keeper(runner, store)
        #expect(!second.reads.finishedUnread(row()), "counted read before the runner is heard from")
        second.adopt(board: runner.board())
        await second.flush()
        #expect(runner.marks[Self.task] == Self.moved)
        #expect(store.loadPending(host: Self.host, workspace: Self.ws).isEmpty)
    }

    @Test func anyAnswerSettlesAMarkEvenOneTheRunnerSkipped() async {
        let runner = Runner()
        runner.skips = [Self.task]
        let store = defaults()
        let k = keeper(runner, store)
        k.adopt(board: runner.board())
        await k.flush()
        k.open(row())
        await k.flush()
        let sends = runner.sent.count
        k.adopt(board: runner.board())
        await k.flush()
        #expect(runner.sent.count == sends, "a skipped mark isn't sent again on every board read")
        #expect(store.loadPending(host: Self.host, workspace: Self.ws).isEmpty)
    }

    @Test func aMarkMadeWhileASendIsOutGoesAfterIt() async {
        let runner = Runner()
        let k = keeper(runner, defaults())
        k.adopt(board: runner.board())
        await k.flush()
        runner.holds = true
        k.open(row())
        k.adopt(board: runner.board())  // a board read, while the send is out: it doesn't wait
        k.open(row("b"))
        for _ in 0..<20 { await Task.yield() }
        #expect(runner.sent.count == 1, "one send at a time, the rest wait their turn")
        runner.release()
        await k.flush()
        #expect(runner.marks[Self.task] == Self.moved)
        #expect(runner.marks["b"] == Self.moved, "the second mark isn't lost behind the first")
    }

    @Test func thePhonesOwnMarksGoUpOnceAndNeverItsFloor() async {
        let runner = Runner()
        let store = defaults()
        // State this phone kept from before the runner kept any: a floor it
        // set itself, and one opened ticket.
        store.save(
            BoardReads(floor: ms(Self.moved - 90_000), opened: [Self.task: ms(Self.moved + 100)]), host: Self.host,
            workspace: Self.ws)
        let k = keeper(runner, store)
        k.adopt(board: runner.board())
        await k.flush()
        #expect(runner.floor == 0, "a phone's floor was never seen as Unread by anyone")
        #expect(runner.marks[Self.task] == Self.moved + 100)
        #expect(store.isUploaded(host: Self.host, workspace: Self.ws))

        let sends = runner.sent.count
        let again = keeper(runner, store)
        again.adopt(board: runner.board())
        await again.flush()
        #expect(runner.sent.count == sends, "once per runner")
    }

    @Test func aFirstLookThePhoneMadeUpIsNeverUploaded() async {
        let runner = Runner()
        let store = defaults()
        // Launch one makes up a first look and never hears from a runner that
        // keeps state (offline, or an older build).
        _ = keeper(runner, store)
        // Launch two does.
        let k = keeper(runner, store)
        k.adopt(board: runner.board())
        await k.flush()
        #expect(runner.sent.isEmpty, "nothing real was kept, so nothing goes up")
        #expect(runner.floor == 0)
    }

    @Test func aFirstLookThePhoneMadeUpDoesNotHideWhatTheRunnerShows() {
        let runner = Runner()
        let now = ms(Self.moved + 90_000_000)  // a day and a bit after the row moved
        let k = keeper(runner, defaults(), now: now)
        #expect(k.reads.floor == now.addingTimeInterval(-86_400), "alone, the last day counts unread")
        k.adopt(board: runner.board())  // the runner's floor is 0: nothing is read
        #expect(k.reads.finishedUnread(row()), "the runner says it's unread")
    }

    @Test func aFloorThePhoneKeptStillStandsOnARunnerThatKeepsState() {
        let runner = Runner()
        let store = defaults()
        store.save(BoardReads(floor: ms(Self.moved + 1000)), host: Self.host, workspace: Self.ws)
        store.save(BoardReads(floor: ms(Self.moved + 2000)), host: Self.host, workspace: Self.ws)  // moved: somebody's doing
        let k = keeper(runner, store)
        k.adopt(board: runner.board())
        #expect(k.reads.floor == ms(Self.moved + 2000))
    }

    @Test func anOlderRunnerKeepsThePhonesOwnStateOnTheDeviceClock() async {
        let runner = Runner()
        let store = defaults()
        store.save(BoardReads(floor: ms(1000)), host: Self.host, workspace: Self.ws)
        let k = keeper(runner, store)
        k.adopt(board: Data(#"{"tasks":[]}"#.utf8))
        #expect(!k.runnerKeepsReads)
        let now = Date(timeIntervalSince1970: 1_900_000_000)
        k.open(row(), now: now)
        await k.flush()
        #expect(runner.sent.isEmpty)
        #expect(store.load(host: Self.host, workspace: Self.ws, now: now).opened[Self.task] == now)
    }

    @Test func aRunnerThatStopsKeepingStateIsKeptOnThePhoneAgain() {
        let runner = Runner()
        let store = defaults()
        let k = keeper(runner, store)
        k.adopt(board: runner.board())
        k.open(row())
        k.adopt(board: Data(#"{"tasks":[]}"#.utf8))
        #expect(!k.runnerKeepsReads)
        #expect(store.load(host: Self.host, workspace: Self.ws, now: Date()).opened[Self.task] == ms(Self.moved))
    }

    @Test func aChangeIsAnnouncedHoweverItCame() async {
        let runner = Runner()
        let k = keeper(runner, defaults())
        k.adopt(board: runner.board())
        var told: [BoardReads] = []
        k.onChange = { told.append($0) }
        k.heard(WireBoardReads(workspaceID: Self.ws, floorMs: Self.moved))
        k.heard(WireBoardReads(workspaceID: Self.ws, floorMs: Self.moved - 1))
        k.open(row("b", movedMs: Self.moved + 10))
        #expect(told.map(\.floor) == [ms(Self.moved), ms(Self.moved)], "a repeat or a lower state says nothing")
        #expect(told.last?.opened["b"] == ms(Self.moved + 10))
    }

    @Test func theArgumentsAreTheFFIs() {
        let raise = ReadsRaise(floor: ms(5000), opened: ["b": ms(7000), "a": ms(6000)])
        let args = raise.rpcArguments(workspace: "w")
        #expect(args["workspace"] as? String == "w")
        #expect(args["floor_ms"] as? Int64 == 5000)
        let opened = args["opened"] as? [[String: Any]]
        #expect(opened?.map { $0["task_id"] as? String } == ["a", "b"])
        #expect(opened?.first?["opened_ms"] as? Int64 == 6000)
        #expect(ReadsRaise(opened: ["a": ms(1)]).rpcArguments(workspace: "w")["floor_ms"] == nil)
    }
}

@Test("The Unread lines say what happened and when, in the Mac's words")
func unreadLinesSayWhen() {
    let now = Date(timeIntervalSince1970: 1_800_000_000)
    func item(_ id: String, _ detail: String?, _ ago: TimeInterval) -> BoardSummary.Item {
        .init(id: id, taskID: "t", key: "k", title: "T", detail: detail, at: now.addingTimeInterval(-ago))
    }
    #expect(item("t/done", nil, 7300).when(now: now) == "Done 2h ago")
    #expect(item("t/created", nil, 200).when(now: now) == "Added 3m ago")
    #expect(item("t/needs_decision", "Needs Decision", 10).when(now: now) == "Needs Decision just now")
    let note = BoardSummary.Activity(
        taskID: "t", key: "k", title: "T", noteID: "n", kind: .finding, text: "x", at: now.addingTimeInterval(-720), more: 2)
    #expect(note.foot(now: now) == "12m ago · +2 more")
}

@Test("Mark All as Read counts a task once, and says it clears every device only when it does")
func markAllReadMessage() {
    let at = Date(timeIntervalSince1970: 1)
    let done = BoardSummary.Item(id: "a/done", taskID: "a", key: "k", title: "T", at: at)
    let note = BoardSummary.Activity(taskID: "a", key: "k", title: "T", noteID: "n", kind: .finding, text: "x", at: at, more: 0)
    let both = BoardSummary(finished: [done], activity: [note])
    #expect(both.taskCount == 1)
    #expect(BoardSummary.markAllReadMessage(tasks: 1, everywhere: false) == "1 task will be marked as read.")
    #expect(BoardSummary.markAllReadMessage(tasks: 68, everywhere: true) == "68 tasks will be marked as read on all your devices.")
}
