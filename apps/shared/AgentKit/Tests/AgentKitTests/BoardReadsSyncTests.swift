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
    #expect(reads.opened["a"] == t0.addingTimeInterval(-100), "the mark can't be raised by nothing, only set to the row's")
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

@Test("What the runner hasn't got is the higher floor and the newer marks, and nothing else")
func raisingOverRunner() {
    let runner = BoardReads(floor: t0, opened: ["a": t0.addingTimeInterval(10)])
    let mine = BoardReads(floor: t0, opened: ["a": t0.addingTimeInterval(10), "b": t0.addingTimeInterval(5)])
    #expect(mine.raising(over: runner) == ReadsRaise(opened: ["b": t0.addingTimeInterval(5)]))
    #expect(runner.raising(over: runner) == nil)
    let floored = BoardReads(floor: t0.addingTimeInterval(100))
    #expect(floored.raising(over: runner) == ReadsRaise(floor: t0.addingTimeInterval(100)))
}

@Test("The arguments are board mark-read's, in the runner's milliseconds")
func raiseArguments() {
    let raise = ReadsRaise(
        floor: Date(timeIntervalSince1970: 2), opened: ["b": Date(timeIntervalSince1970: 1.5), "a": Date(timeIntervalSince1970: 3)])
    #expect(raise.arguments == ["--task", "a:3000", "--task", "b:1500", "--floor", "2000"])
}
