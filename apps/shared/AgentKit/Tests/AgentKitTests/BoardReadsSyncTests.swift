import Foundation
import Testing

@testable import AgentKit

// Read state kept on the runner (ov-113): its wire shape, and merging by max.

private let t0 = Date(timeIntervalSince1970: 1_800_000_000)

private func row(_ id: String, moved ago: TimeInterval) -> TaskRow {
    TaskRow(
        id: id, key: "k-\(id)", title: id, status: .done, statusSince: t0.addingTimeInterval(-ago),
        createdAt: nil, updatedAt: t0.addingTimeInterval(-ago))
}

@Test("The runner's reads decode from a board and from a bare state")
func wireReadsDecode() throws {
    let board = Data(
        #"{"tasks":[],"reads":{"workspace_id":"w1","floor_ms":1000,"opened":[{"task_id":"a","opened_ms":5000},{"task_id":"b","opened_ms":500}]}}"#
            .utf8)
    let wire = try #require(WireBoardReads.decode(board: board))
    #expect(wire.reads.floor == Date(timeIntervalSince1970: 1))
    #expect(wire.reads.opened == ["a": Date(timeIntervalSince1970: 5)], "a mark under the floor says nothing")
    #expect(WireBoardReads.decode(board: Data(#"{"tasks":[]}"#.utf8)) == nil, "an old runner sends none")
    #expect(WireBoardReads.decode(board: Data(#"{"tasks":[],"reads":{"floor_ms":"x"}}"#.utf8)) == nil)
}

@Test("Merging takes each mark and the floor at their later, and never lowers one")
func mergeIsMax() {
    let a = BoardReads(floor: t0, opened: ["x": t0.addingTimeInterval(50), "y": t0.addingTimeInterval(10)])
    let b = BoardReads(floor: t0.addingTimeInterval(20), opened: ["x": t0.addingTimeInterval(30), "z": t0.addingTimeInterval(70)])
    let merged = a.merged(with: b)
    #expect(merged.floor == t0.addingTimeInterval(20))
    #expect(merged.opened == ["x": t0.addingTimeInterval(50), "z": t0.addingTimeInterval(70)], "y fell under the floor")
    #expect(a.merged(with: b) == b.merged(with: a), "order can't matter")
}

@Test("Opening through the runner's time uses no clock of this device")
func openSeenThroughIgnoresDeviceClock() {
    var reads = BoardReads(floor: t0.addingTimeInterval(-3600))
    let r = row("a", moved: 100)
    reads.open(r, seenThrough: t0.addingTimeInterval(-40))
    #expect(reads.opened["a"] == t0.addingTimeInterval(-40), "the later of the row and the newest note, not now")
    reads.open(r, seenThrough: nil)
    #expect(reads.opened["a"] == t0.addingTimeInterval(-40), "a mark already held is never lowered")
    reads.open(row("b", moved: 100), seenThrough: nil)
    #expect(reads.opened["b"] == t0.addingTimeInterval(-100), "a first mark is the row's own last word")
}

@Test("Mark All as Read through the runner's time raises the floor and drops the marks it passes")
func markAllSeenThrough() {
    var reads = BoardReads(floor: t0.addingTimeInterval(-3600), opened: ["a": t0.addingTimeInterval(-500)])
    reads.markAllRead(rows: [row("a", moved: 100), row("b", moved: 300)], seenThrough: t0.addingTimeInterval(-20))
    #expect(reads.floor == t0.addingTimeInterval(-20))
    #expect(reads.opened.isEmpty)
    var again = reads
    again.markAllRead(rows: [], seenThrough: nil)
    #expect(again.floor == reads.floor, "the floor only rises")
}

@Test("Marks owed merge by max, apply to what is read, and leave only what was not sent")
func owedMarks() {
    let owed = ReadsRaise(floor: t0, opened: ["a": t0.addingTimeInterval(10)])
    let more = ReadsRaise(floor: t0.addingTimeInterval(5), opened: ["a": t0.addingTimeInterval(4), "b": t0.addingTimeInterval(9)])
    let both = owed.merging(more)
    #expect(both == ReadsRaise(floor: t0.addingTimeInterval(5), opened: ["a": t0.addingTimeInterval(10), "b": t0.addingTimeInterval(9)]))
    let sent = ReadsRaise(floor: t0.addingTimeInterval(5), opened: ["a": t0.addingTimeInterval(10)])
    #expect(both.without(sent) == ReadsRaise(opened: ["b": t0.addingTimeInterval(9)]), "a mark raised while sending stays owed")
    let raised = both.merging(ReadsRaise(opened: ["a": t0.addingTimeInterval(20)]))
    #expect(raised.without(sent).opened["a"] == t0.addingTimeInterval(20))
    #expect(both.without(both).isEmpty)
    let base = BoardReads(floor: t0.addingTimeInterval(-100))
    #expect(both.applied(to: base).floor == t0.addingTimeInterval(5))
    #expect(both.applied(to: base).opened["b"] == t0.addingTimeInterval(9))
}

@Test("Owed marks are kept in defaults, with and without a floor")
func owedMarksPersist() {
    let defaults = UserDefaults(suiteName: "ov113-\(UUID().uuidString)")!
    let store = DefaultsBoardReads(defaults)
    #expect(store.loadPending(host: "h", workspace: "w").isEmpty)
    let owed = ReadsRaise(floor: Date(timeIntervalSince1970: 5), opened: ["a": Date(timeIntervalSince1970: 9)])
    store.savePending(owed, host: "h", workspace: "w")
    #expect(store.loadPending(host: "h", workspace: "w") == owed)
    store.savePending(ReadsRaise(), host: "h", workspace: "w")
    #expect(store.loadPending(host: "h", workspace: "w").isEmpty)
    #expect(!store.hasState(host: "h", workspace: "w"))
    store.save(BoardReads(floor: t0), host: "h", workspace: "w")
    #expect(store.hasState(host: "h", workspace: "w"))
}

@Test("The arguments are board mark-read's, in the runner's milliseconds")
func raiseArguments() {
    let raise = ReadsRaise(
        floor: Date(timeIntervalSince1970: 2), opened: ["b": Date(timeIntervalSince1970: 1.5), "a": Date(timeIntervalSince1970: 3)])
    #expect(raise.arguments == ["--task", "a:3000", "--task", "b:1500", "--floor", "2000"])
}
