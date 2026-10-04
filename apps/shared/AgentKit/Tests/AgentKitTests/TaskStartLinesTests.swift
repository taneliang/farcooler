import Foundation
import Testing

@testable import AgentKit

// What a card says about who is on it and when it starts, read from the ONE
// fixture Android's `TaskStartLinesTest` reads too (`task_start_lines.json`):
// a row as `task list --json` sends it, plus the panes and the clock, in; the
// control's words, the quiet line and the Waiting on sentence, out. Whole
// strings, so the two phones cannot drift by a word.

private struct Pane: TaskBoardPane {
    var boardTaskID: String?
    var boardState: String = "running"
    var runsAgent: Bool = true
}

private struct Fixture: Decodable {
    var now: Int64
    var cases: [Case]

    struct Case: Decodable {
        var name: String
        var panes: Int
        var recordsTasks: Bool
        var task: AnyJSON
        var expect: Expect

        enum CodingKeys: String, CodingKey {
            case name, panes, task, expect
            case recordsTasks = "records_tasks"
        }
    }

    struct Expect: Decodable {
        var chip: [String: String]?
        var line: String?
        var blocked: String?
    }
}

/// A task object, kept as JSON so it goes through the same decoder the app uses.
private struct AnyJSON: Decodable {
    var data: Data

    init(from decoder: Decoder) throws {
        let value = try JSONValue(from: decoder)
        data = try JSONSerialization.data(withJSONObject: value.object)
    }
}

private enum JSONValue: Decodable {
    case null, bool(Bool), number(Double), string(String), array([JSONValue]), object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null } else if let v = try? c.decode(Bool.self) {
            self = .bool(v)
        } else if let v = try? c.decode(Int64.self) {
            self = .number(Double(v))
        } else if let v = try? c.decode(Double.self) {
            self = .number(v)
        } else if let v = try? c.decode(String.self) {
            self = .string(v)
        } else if let v = try? c.decode([JSONValue].self) {
            self = .array(v)
        } else {
            self = .object(try c.decode([String: JSONValue].self))
        }
    }

    var object: Any {
        switch self {
        case .null: return NSNull()
        case .bool(let v): return v
        case .number(let v): return v == v.rounded() ? NSNumber(value: Int64(v)) : NSNumber(value: v)
        case .string(let v): return v
        case .array(let v): return v.map(\.object)
        case .object(let v): return v.mapValues(\.object)
        }
    }
}

private func fixture() throws -> Fixture {
    let url = try #require(Bundle.module.url(forResource: "task_start_lines", withExtension: "json"))
    return try JSONDecoder().decode(Fixture.self, from: Data(contentsOf: url))
}

private func row(_ task: AnyJSON) throws -> TaskRow {
    let wire = try JSONDecoder().decode(WireTask.self, from: task.data)
    let status = try #require(TaskStatus(rawValue: wire.status))
    return wire.row(status: status)
}

@Test func theFixtureHasCasesToCheck() throws {
    #expect(try fixture().cases.count > 30)
}

@Test func theControlSaysTheSameWordsOnEveryCase() throws {
    var wrong: [String] = []
    for c in try fixture().cases {
        let r = try row(c.task)
        let panes = (0..<c.panes).map { _ in Pane(boardTaskID: r.id) }
        let got = r.agentPresence(
            livePanes: r.livePanes(in: panes).count, runnerRecordsTasks: c.recordsTasks
        ).title
        if got != c.expect.chip?["apple"] { wrong.append("\(c.name): \(got ?? "nil") != \(c.expect.chip?["apple"] ?? "nil")") }
    }
    #expect(wrong.isEmpty, "\(wrong)")
}

@Test func theBlocksSayTheSameSentenceOnEveryCase() throws {
    var wrong: [String] = []
    for c in try fixture().cases {
        let got = try row(c.task).blockedSummary
        if got != c.expect.blocked { wrong.append("\(c.name): \(got ?? "nil") != \(c.expect.blocked ?? "nil")") }
    }
    #expect(wrong.isEmpty, "\(wrong)")
}

@Test func theStartLineSaysTheSameSentenceOnEveryCase() throws {
    let fx = try fixture()
    let now = Date(timeIntervalSince1970: Double(fx.now) / 1000)
    var wrong: [String] = []
    for c in fx.cases {
        let got = try row(c.task).startLine(
            at: now, speaksOfAgents: c.recordsTasks, timeZone: TimeZone(identifier: "UTC")!,
            locale: Locale(identifier: "en_US"))
        if got != c.expect.line { wrong.append("\(c.name): \(got ?? "nil") != \(c.expect.line ?? "nil")") }
    }
    #expect(wrong.isEmpty, "\(wrong)")
}

/// A done blocker stops counting on the opened card too: the edge is history.
@Test func aBlockerThatIsDoneOrCanceledIsNotWaitedOn() {
    func blocker(_ id: String, _ key: String, _ status: TaskStatus) -> TaskRow {
        TaskRow(id: id, key: key, title: key, status: status, statusSince: .now)
    }
    let board = TaskBoardModel(columns: [
        TaskBoardColumn(status: .done, rows: [blocker("d", "ov-36", .done)]),
        TaskBoardColumn(status: .cancelled, rows: [blocker("c", "ov-37", .cancelled)]),
        TaskBoardColumn(status: .inProgress, rows: [blocker("p", "ov-38", .inProgress)]),
    ])
    let raw = [
        RawTaskBlock(blockedBy: "d", short: "d", reason: "after the rename"),
        RawTaskBlock(blockedBy: "c", short: "c", reason: ""),
        RawTaskBlock(blockedBy: "p", short: "p", reason: "same files"),
        RawTaskBlock(blockedBy: "gone", short: "0198f2c0", reason: ""),
    ]
    #expect(board.resolvingBlocks(raw).map(\.key) == ["ov-38", "0198f2c0"])
}
