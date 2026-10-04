import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// What a Mac row says about who is on a task and when it starts (ov-212,
/// ov-213): the lead's order, "No Agent" only when nothing explains the stop,
/// the subagent's sentence, and parity with the phones' shared fixture.
struct TaskRowMetaWorkersTests {
    private static let now = Date(timeIntervalSince1970: 1_759_400_000)

    private static func worker(
        _ state: TaskWorkerState = .running, minutes: Double = 12, doing: String = "Running cargo test",
        orchestrator: String? = nil, model: String = ""
    ) -> TaskWorker {
        TaskWorker(
            harness: "claude", state: state, startedAt: now.addingTimeInterval(-minutes * 60),
            endedAt: state == .finished ? now.addingTimeInterval(-minutes * 60) : nil, doing: doing,
            orchestratorTerminalID: orchestrator, model: model)
    }

    private static func row(
        _ status: TaskStatus = .inProgress, workers: [TaskWorker] = [], wait: TaskWait? = nil,
        blocked: Bool = false, staleDays: Double = 0
    ) -> TaskRow {
        let then = now.addingTimeInterval(-staleDays * 86_400 - 60)
        var row = TaskRow(
            id: "t", key: "ov-1", title: "A task", status: status, statusSince: then,
            blockedBy: blocked ? [TaskBlockRef(key: "ov-9", reason: "")] : [], createdAt: then, updatedAt: then)
        row.workers = workers
        row.wait = wait
        return row
    }

    /// The row as the board draws it: presence, then the meta line.
    private static func text(_ row: TaskRow, panes: Int = 0) -> String {
        let presence = row.agentPresence(livePanes: panes, runnerRecordsTasks: true)
        return TaskRowMeta.line(
            row, agent: TaskRowMeta.agent(live: [], presence: presence),
            startLine: row.startLine(at: now), at: now
        ).text
    }

    @Test("A running subagent is an agent: no No Agent, and the line says who and for how long")
    func runningSubagent() {
        let line = Self.text(Self.row(workers: [Self.worker()]))
        #expect(line == "Claude subagent working, 12 min · Running cargo test")
        #expect(!line.contains("No Agent"))
    }

    @Test("A card waiting to build is not an alarm, and its subagent's word is kept")
    func buildWaitExplainsTheStop() {
        let waiting = Self.row(wait: .inLine(.build, position: 2))
        #expect(Self.text(waiting) == "Waiting to build, 2nd in line")
        let both = Self.row(workers: [Self.worker()], wait: .inLine(.build, position: 2))
        #expect(Self.text(both) == "Waiting to build, 2nd in line · Subagent")
    }

    @Test("Nobody on a started card, and no reason, is still No Agent")
    func noAgentRemains() {
        #expect(Self.text(Self.row(staleDays: 0)).contains("No Agent"))
    }

    @Test("A queued task says why, and a finished subagent speaks beside the wait")
    func queuedWithAReason() {
        let queued = Self.row(.todo, wait: .inLine(.agent, position: 1))
        #expect(Self.text(queued) == "Next to start")
        let finished = Self.row(
            workers: [Self.worker(.finished, minutes: 12)], wait: .inLine(.build, position: 2))
        #expect(Self.text(finished) == "Waiting to build, 2nd in line · Subagent finished 12 min ago")
    }

    @Test("The lead goes: what blocks it, when it starts, how long it sat, the ask, its time")
    func leadOrder() {
        let all = Self.row(
            .todo, wait: .inLine(.agent, position: 1), blocked: true)
        #expect(TaskRowMeta.line(all, startLine: all.startLine(at: Self.now), at: Self.now).lead == "Waiting on ov-9")
        let started = Self.row(wait: .inLine(.build, position: 1), staleDays: 3)
        #expect(
            TaskRowMeta.line(started, startLine: started.startLine(at: Self.now), at: Self.now).lead
                == "Builds next")
        let stale = Self.row(staleDays: 3)
        #expect(TaskRowMeta.line(stale, startLine: stale.startLine(at: Self.now), at: Self.now).lead == "No movement for 3d")
        let ask = Self.row(.needsDecision)
        #expect(TaskRowMeta.line(ask, startLine: ask.startLine(at: Self.now), at: Self.now).lead == "Answer to unblock")
    }

    @Test("A blocked task reads in the accent; a plain wait stays quiet")
    func onlyBlocksAreColored() {
        let blocked = Self.row(.todo, blocked: true)
        #expect(TaskRowMeta.line(blocked, startLine: blocked.startLine(at: Self.now), at: Self.now).tone == .attention)
        let waiting = Self.row(wait: .inLine(.build, position: 2), staleDays: 0)
        #expect(TaskRowMeta.line(waiting, startLine: waiting.startLine(at: Self.now), at: Self.now).tone == .quiet)
    }

    @Test("The Subagent control goes to the orchestrator's pane only while that pane is live")
    func orchestratorPane() {
        func terminal(_ id: String, state: String) -> Terminal {
            Terminal(id: id, short: id, title: "claude", preset: "claude", state: state, epoch: 0)
        }
        func worktree(_ terminals: [Terminal]) -> Worktree {
            Worktree(
                id: "w", short: "w", task: "main", branch: "main", repository: "r", host: "",
                path: "/tmp/w", state: "active", terminals: terminals)
        }
        let row = Self.row(workers: [Self.worker(orchestrator: "o1")])
        let live = worktree([terminal("o1", state: "running")])
        #expect(TaskWorkers.orchestrator(of: row, in: [live])?.id == "o1")
        let gone = worktree([terminal("o1", state: "exited")])
        #expect(TaskWorkers.orchestrator(of: row, in: [gone]) == nil)
        #expect(TaskWorkers.orchestrator(of: Self.row(workers: [Self.worker()]), in: [live]) == nil)
    }

    @Test("The task view names a subagent's model, and only there")
    func modelInTheTaskView() {
        let row = Self.row(workers: [Self.worker(doing: "", model: "opus")])
        #expect(TaskWorkers.lines(of: row, at: Self.now) == ["Claude subagent working, 12 min · Opus"])
        #expect(Self.text(row) == "Claude subagent working, 12 min")
    }

    @Test("The task view lists each subagent in AgentKit's words, open ones first")
    func workersSection() {
        let row = Self.row(workers: [
            Self.worker(.finished, minutes: 30), Self.worker(.running, minutes: 5, doing: ""),
        ])
        #expect(
            TaskWorkers.lines(of: row, at: Self.now)
                == ["Claude subagent working, 5 min", "Subagent finished 30 min ago"])
    }

    // MARK: The phones' fixture

    private struct Case {
        var name: String
        var panes: Int
        var recordsTasks: Bool
        var task: [String: Any]
        var chip: String?
        var line: String?
        var blocked: String?
    }

    private static func fixture() throws -> (now: Date, cases: [Case]) {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("shared/AgentKit/Tests/AgentKitTests/task_start_lines.json")
        let json = try #require(
            JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
        let ms = try #require((json["now"] as? NSNumber)?.int64Value)
        let cases = try #require(json["cases"] as? [[String: Any]]).map { c -> Case in
            let expect = c["expect"] as? [String: Any] ?? [:]
            return Case(
                name: c["name"] as? String ?? "", panes: c["panes"] as? Int ?? 0,
                recordsTasks: c["records_tasks"] as? Bool ?? true, task: c["task"] as? [String: Any] ?? [:],
                chip: (expect["chip"] as? [String: Any])?["apple"] as? String,
                line: expect["line"] as? String, blocked: expect["blocked"] as? String)
        }
        return (Date(timeIntervalSince1970: Double(ms) / 1000), cases)
    }

    private static func read(_ task: [String: Any]) throws -> TaskRow {
        let data = try JSONSerialization.data(withJSONObject: ["tasks": [task]])
        return try #require(TaskBoardModel.decode(data).rows.first)
    }

    @Test("The Mac says every case of the shared fixture word for word")
    func sharedFixtureParity() throws {
        let (now, cases) = try Self.fixture()
        #expect(cases.count > 30)
        var wrong: [String] = []
        for c in cases {
            let row = try Self.read(c.task)
            let chip = row.agentPresence(livePanes: c.panes, runnerRecordsTasks: c.recordsTasks).title
            let line = row.startLine(
                at: now, speaksOfAgents: c.recordsTasks, timeZone: TimeZone(identifier: "UTC")!,
                locale: Locale(identifier: "en_US"))
            if chip != c.chip { wrong.append("\(c.name): chip \(chip ?? "nil") != \(c.chip ?? "nil")") }
            if line != c.line { wrong.append("\(c.name): line \(line ?? "nil") != \(c.line ?? "nil")") }
            if row.blockedSummary != c.blocked { wrong.append("\(c.name): blocked") }
            // And on the row itself, the lead is that sentence unless a block leads.
            if c.blocked == nil, let expected = c.line {
                let lead = TaskRowMeta.line(row, startLine: line, at: now).lead
                if lead != expected { wrong.append("\(c.name): lead \(lead ?? "nil")") }
            }
        }
        #expect(wrong.isEmpty, "\(wrong)")
    }
}
