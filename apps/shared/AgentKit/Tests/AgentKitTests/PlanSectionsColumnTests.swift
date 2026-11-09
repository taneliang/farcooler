import Foundation
import Testing

@testable import AgentKit

/// The plan column (ov-867, C10f): the facts line, Initiatives from themes,
/// the pipeline of agents with their tickets under them, and the footer
/// counts. The Rust core decides it (ov-1016) and its tests pin the rules
/// (`plan_view/column_tests.rs`); here the golden decodes into every slot the
/// apps draw, and the runner's own plan reads through the bridge. Named for
/// `--filter PlanSections`.
struct PlanSectionsColumnTests {
    static let now = PlanModelTests.now
    static let minute: Int64 = 60_000

    /// An agent on the wire: `tickets` are the tasks it works, as `build`, `review` or `land`.
    static func agent(
        _ id: String, _ role: String, tickets: [String] = [], work: String = "build", doing: String = "", title: String = "",
        terminal: String? = nil, started: Int64
    ) -> [String: Any] {
        [
            "id": id, "short": id, "role": role, "harness": "claude", "model": "sonnet", "title": title,
            "terminal": terminal as Any? ?? NSNull(), "subagent_id": "", "parent_terminal": NSNull(), "stop_asked_at": NSNull(),
            "tickets": tickets.map {
                ["task": $0, "key": "ov-\($0.dropFirst())", "role": work, "started_at": now - started * minute]
            },
            "doing_now": doing, "working": true, "started_at": now - started * minute, "last_at": now,
            "spend": ["input_tokens": 0, "runs": 0, "unmeasured_agents": 0],
        ]
    }

    static func plan(
        themes: [[String: Any]] = [], lines: [[String: Any]], landings: [[String: Any]] = [], agents: [[String: Any]] = [],
        cards: [[String: Any]] = [], counts: [String: Int]? = nil
    ) throws -> PlanModel {
        var object: [String: Any] = [
            "now_ms": now, "themes": themes, "lines": lines, "cards": cards, "landings": landings, "agents": agents,
        ]
        if let counts { object["board_counts"] = counts }
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
    }

    static func card(_ task: String, _ title: String) -> [String: Any] {
        ["task": task, "key": "ov-\(task.dropFirst())", "title": title, "status": "In Progress"]
    }

    // Two rules the Rust tests don't cover yet, kept here through the bridge.

    @Test("the golden's column decodes into every slot: facts, initiatives, the pipeline and the footer")
    func decodesTheGolden() throws {
        let column = try PlanViewTests.view().column
        #expect(column.facts.map(\.kind) == [.needsYou, .stuck] && column.facts.map(\.id) == ["needsYou", "stuck"])
        #expect(column.facts[0].runs == [.init(text: "2", strong: true), .init(text: " need you", strong: false)])
        #expect(column.facts[0].text == "2 need you" && column.facts[0].attention)
        #expect(column.initiativeCount == 3 && column.initiatives.map(\.progress) == ["2 of 5", "0 of 3", "0 of 5"])
        let marketing = try #require(column.initiatives.first { $0.name == "Marketing" })
        #expect(marketing.asksOwner && marketing.next == "Waiting on you: How do we ship the site?" && marketing.fraction == 0)
        #expect(column.initiatives[0].fraction == 0.4)

        #expect(column.pipeline.map(\.stage) == [.stuck, .building, .reviewing, .landing])
        #expect(column.pipeline.map(\.title) == ["Stuck", "Building", "Reviewing", "Landing"])
        #expect(column.pipeline[3].agents.map(\.role) == [.lander])
        #expect(column.quietInitiatives.isEmpty && column.quietLine == nil)
        let stuck = column.pipeline[0]
        #expect(stuck.agents.isEmpty && stuck.count == 2 && stuck.preview.map(\.key) == ["ov-482", "ov-474"])
        #expect(stuck.unattendedWords == "2 tickets with no agent on them" && stuck.hidden == 0)
        let building = column.pipeline[1]
        let builder = try #require(building.agents.first)
        #expect(builder.role == .builder && builder.role.symbol == "hammer" && builder.name == "Builder")
        #expect(builder.doing == "Working" && !builder.doingIsTrouble && builder.working)
        #expect(builder.elapsed == 1_320_000 && builder.time == "22 min" && builder.more == 1 && builder.ticket == "t-ov-310")
        let ticket = try #require(builder.tickets.first)
        #expect(ticket.title == "The themes grid" && ticket.initiative == "Trustworthy agent view" && ticket.initiativeId == "th-3")
        #expect(ticket.stage == .building && ticket.since == 1_799_996_400_000 && ticket.id == "t-ov-310")
        #expect(building.preview.isEmpty && building.hidden == 1 && building.unattendedWords == "1 more with no builder running")

        #expect(column.agentWords == "2 agents")
        let waiting = try #require(column.footer.first)
        #expect(waiting.kind == .waiting && waiting.id == "waiting" && waiting.taskStatus == .todo && waiting.count == 5)
        #expect(waiting.words == "5 tickets waiting for a builder")
        #expect(column.waiting.count == 5)
    }

    @Test("a footer count's snake-case kind decodes into the name the apps route and identify it by")
    func footerKinds() throws {
        let data = Data(#"{"kind": "not_started", "count": 22, "status": "backlog", "words": "22 not started"}"#.utf8)
        let count = try JSONDecoder().decode(WireCount.self, from: data)
        #expect(count.kind == .notStarted && count.kind.name == "notStarted" && count.taskStatus == .backlog)
        let fact = try JSONDecoder().decode(WireFactKind.self, from: Data(#""next_landing""#.utf8))
        #expect(fact == .nextLanding)
    }

    @Test("a ticket that needs you puts its agent in Stuck too, with the agent's own words")
    func needsYouIsStuck() throws {
        let plan = try Self.plan(
            lines: [PlanModelTests.line("t1", "needs_you")],
            agents: [Self.agent("a", "builder", tickets: ["t1"], doing: "Waiting on a decision", started: 4)])
        let column = try #require(WireColumn.of(plan))
        #expect(column.pipeline.map(\.stage) == [.stuck])
        #expect(column.pipeline[0].agents[0].doing == "Waiting on a decision" && !column.pipeline[0].agents[0].doingIsTrouble)
    }

    @Test("a live landing carrying no ticket yet is not named as the next landing")
    func emptyLanding() throws {
        let plan = try Self.plan(
            lines: [PlanModelTests.line("t1", "landed", landing: 9), PlanModelTests.line("t2", "building")],
            landings: [PlanModelTests.landing(9, "gating", tickets: ["t1"], phase: "gating_locally", since: Self.now - 20 * Self.minute)])
        let column = try #require(WireColumn.of(plan))
        #expect(!column.facts.contains { $0.kind == .nextLanding }, "not \"Next landing: 0 tickets, 20 min\"")
    }

    @Test("the runner's own plan.json reads as a column, with nothing invented")
    func fixture() throws {
        let plan = try PlanModelTests.fixture()
        let column = try #require(WireColumn.of(plan))
        for group in column.pipeline {
            for agent in group.agents {
                #expect(!agent.doing.isEmpty && !agent.name.isEmpty)
                #expect(plan.agents.contains { $0.id == agent.id }, "\(agent.id) is a real agent")
            }
        }
        #expect(column.facts.contains { $0.kind == .done })
    }

    @Test("clicking a Needs You row quotes it: a task by key and title, else the words")
    func quote() {
        #expect(WireColumn.quote(key: "ov-1", title: "Site copy") == "About ov-1 (\u{201C}Site copy\u{201D}): ")
        #expect(WireColumn.quote(key: nil, title: "Allow\nBash?") == "About \u{201C}Allow Bash?\u{201D}: ")
    }
}
