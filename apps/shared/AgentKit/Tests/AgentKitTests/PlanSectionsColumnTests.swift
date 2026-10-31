import Foundation
import Testing

@testable import AgentKit

/// The plan column (ov-867, C10f) on today's wire: the facts line,
/// Initiatives from themes, the pipeline of agents with their tickets under
/// them, and the footer counts. Named for `--filter PlanSections`.
struct PlanSectionsColumnTests {
    static let now = PlanModelTests.now
    static let minute: Int64 = 60_000

    static func agent(
        _ id: String, _ role: String, lane: String? = nil, train: String? = nil, label: String = "", terminal: String? = nil,
        started: Int64
    ) -> [String: Any] {
        [
            "id": id, "role": role, "harness": "claude", "model": "sonnet", "label": label,
            "lane": lane.map { ["id": "lane-\($0)", "name": $0] } as Any? ?? NSNull(),
            "train": train.map { ["id": "train-\($0)", "title": $0] } as Any? ?? NSNull(), "card": NSNull(),
            "terminal": terminal as Any? ?? NSNull(), "working": true, "started_at": now - started * minute, "last_at": now,
            "spend": ["input_tokens": 0, "runs": 0, "unmeasured_agents": 0],
        ]
    }

    static func plan(
        themes: [[String: Any]] = [], lanes: [[String: Any]], trains: [[String: Any]] = [], agents: [[String: Any]] = [],
        cards: [[String: Any]] = [], counts: [String: Int]? = nil
    ) throws -> PlanModel {
        var object: [String: Any] = [
            "now_ms": now, "themes": themes, "lanes": lanes, "order": [], "cards": cards, "trains": trains, "agents": agents,
        ]
        if let counts { object["board_counts"] = counts }
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
    }

    static func card(_ task: String, _ title: String) -> [String: Any] {
        ["task": task, "key": "ov-\(task.dropFirst())", "title": title, "status": "In Progress"]
    }

    @Test("each agent is one card in its lane's stage, with the lane's tickets under it and each ticket's own initiative")
    func agentsCarryTheirTickets() throws {
        var web = PlanModelTests.theme("Web client", cards: ["t1"], ordinal: 1)
        web["outcome"] = "You use Far Cooler from any browser"
        let mobile = PlanModelTests.theme("Mobile experience", cards: ["t2", "t3"], ordinal: 2)
        let plan = try Self.plan(
            themes: [web, mobile],
            lanes: [
                PlanModelTests.lane("b1", "building", cards: ["t1", "t2"]),
                PlanModelTests.lane("r1", "review", cards: ["t3"]),
            ],
            agents: [
                Self.agent("pane:orchestrator", "orchestrator", started: 300),
                Self.agent("agent:b", "build", lane: "b1", label: "Reading the chat list's scroll code", terminal: "term-b", started: 30),
                Self.agent("agent:r", "review", lane: "r1", started: 8),
            ],
            cards: [Self.card("t1", "A paired browser starts after an update"), Self.card("t2", "Chat scrolling stutters")])
        let column = PlanColumn(plan: plan)
        #expect(column.pipeline.map(\.stage) == [.building, .reviewing], "empty stages are left out")
        let builder = try #require(column.pipeline.first?.agents.first)
        #expect(builder.name == "Builder" && builder.role == .builder && builder.terminal == "term-b")
        #expect(builder.doing == "Reading the chat list's scroll code" && builder.time == "30 min")
        #expect(builder.tickets.map(\.title) == ["A paired browser starts after an update", "Chat scrolling stutters"])
        #expect(builder.tickets.map(\.initiative) == ["Web client", "Mobile experience"], "the initiative is the ticket's")
        let reviewer = try #require(column.pipeline.last?.agents.first)
        #expect(reviewer.name == "Reviewer" && reviewer.tickets.map(\.key) == ["ov-3"], "no title anywhere: the key")
        #expect(column.agentWords == "2 agents", "the orchestrator isn't in the pipeline")
    }

    @Test("several agents of one role are numbered in the order they started")
    func numbering() throws {
        let plan = try Self.plan(
            lanes: [PlanModelTests.lane("a", "building", cards: ["t1"]), PlanModelTests.lane("b", "building", cards: ["t2"])],
            agents: [Self.agent("agent:late", "build", lane: "a", started: 5), Self.agent("agent:early", "build", lane: "b", started: 40)])
        let names = PlanColumn(plan: plan).pipeline[0].agents.map { "\($0.id)=\($0.name)" }
        #expect(Set(names) == ["agent:early=Builder 1", "agent:late=Builder 2"])
    }

    @Test("a stale lane's agent is Stuck and says why in amber; the facts line counts it with the longest time")
    func stuck() throws {
        let plan = try Self.plan(
            lanes: [
                PlanModelTests.lane("s", "building", cards: ["t1"], since: Self.now - 150 * Self.minute, stale: true),
                PlanModelTests.lane("ok", "building", cards: ["t2"]),
            ],
            agents: [Self.agent("agent:s", "build", lane: "s", label: "Running the tests", started: 47), Self.agent("agent:ok", "build", lane: "ok", started: 3)])
        let column = PlanColumn(plan: plan, needsYou: 2)
        let stuck = try #require(column.pipeline.first)
        #expect(stuck.stage == .stuck && stuck.agents.map(\.id) == ["agent:s"])
        #expect(stuck.agents[0].doing == "No move in 2 h" && stuck.agents[0].doingIsTrouble)
        #expect(column.facts.map(\.text) == ["2 need you", "1 stuck, 47 min"])
        #expect(column.facts.first { $0.kind == .stuck }?.attention == true)
    }

    @Test("tickets with no agent on them are said under their stage, never given an invented agent")
    func unattended() throws {
        let plan = try Self.plan(
            lanes: [
                PlanModelTests.lane("r1", "review", cards: ["t1"]), PlanModelTests.lane("r2", "review", cards: ["t2", "t3"]),
                PlanModelTests.lane("b", "building", cards: ["t4"]),
            ],
            agents: [Self.agent("agent:r", "review", lane: "r1", started: 8)])
        let column = PlanColumn(plan: plan)
        let reviewing = try #require(column.pipeline.first { $0.stage == .reviewing })
        #expect(reviewing.count == 3 && reviewing.unattended.count == 2)
        #expect(reviewing.unattendedWords == "2 more waiting for review" && reviewing.agents.last?.more == 2)
        let building = try #require(column.pipeline.first { $0.stage == .building })
        #expect(building.agents.isEmpty && building.unattendedWords == "1 ticket with no builder running")
        #expect(column.agentCount == 1)
    }

    @Test("a ticket is on one agent even when several claim its lane: the lander first, then the first builder")
    func eachTicketOnce() throws {
        var train = PlanTrainsTests.train("integ-9", "gating", lanes: ["l1"])
        train["phase"] = "gating_locally"
        let plan = try Self.plan(
            lanes: [
                PlanModelTests.lane("l1", "landing", cards: ["t1"], train: "integ-9"),
                PlanModelTests.lane("b", "building", cards: ["t2"]),
            ],
            trains: [train],
            agents: [
                Self.agent("agent:b1", "build", lane: "l1", started: 9), Self.agent("agent:i", "integrator", train: "integ-9", started: 20),
                Self.agent("agent:b2", "build", lane: "b", started: 8), Self.agent("agent:b3", "build", lane: "b", started: 7),
            ])
        let column = PlanColumn(plan: plan)
        let all = column.pipeline.flatMap { $0.agents.flatMap { $0.tickets.map(\.task) } + $0.unattended.map(\.task) }
        #expect(all.sorted() == ["t1", "t2"], "each once: \(all)")
        #expect(column.pipeline.flatMap(\.agents).first { $0.id == "agent:i" }?.tickets.map(\.task) == ["t1"], "the lander has its train's")
        #expect(column.pipeline.flatMap(\.agents).first { $0.id == "agent:b2" }?.tickets.map(\.task) == ["t2"])
        #expect(column.pipeline.flatMap(\.agents).first { $0.id == "agent:b3" }?.tickets.isEmpty == true)
        #expect(column.pipeline.map(\.count).reduce(0, +) == 2)
    }

    @Test("an empty plan has an empty pipeline, no initiatives, no footer and no facts")
    func emptyPlan() throws {
        let column = PlanColumn(plan: try Self.plan(lanes: []))
        #expect(column.pipeline.isEmpty && column.initiatives.isEmpty && column.quietInitiatives.isEmpty)
        #expect(column.footer.isEmpty && column.facts.isEmpty)
    }

    @Test("the footer leaves out a cancelled ticket on a queued lane, and Needs You is amber above zero")
    func footerAndAttention() throws {
        let plan = try Self.plan(lanes: [PlanModelTests.lane("q", "queued", cards: ["t1", "t2"])])
        let column = PlanColumn(plan: plan, needsYou: 1, statuses: ["t2": .cancelled])
        #expect(column.footer.first { $0.kind == .waiting }?.count == 1)
        #expect(column.facts.first { $0.kind == .needsYou }?.attention == true)
    }

    @Test("a live train's integrator is the lander over its landing tickets; the next landing is on the facts line")
    func lander() throws {
        var train = PlanTrainsTests.train("integ-9", "gating", lanes: ["l1", "l2", "l3"])
        train["phase"] = "gating_locally"
        train["phase_since"] = Self.now - 14 * Self.minute
        let plan = try Self.plan(
            lanes: [
                PlanModelTests.lane("l1", "landing", cards: ["t1"], train: "integ-9"),
                PlanModelTests.lane("l2", "landing", cards: ["t2", "t3"], train: "integ-9"),
                PlanModelTests.lane("l3", "building", cards: ["t4"], train: "integ-9"),
            ],
            trains: [train], agents: [Self.agent("agent:i", "integrator", train: "integ-9", started: 20)],
            counts: ["backlog": 20, "todo": 2, "needs_decision": 0, "in_progress": 5, "in_review": 3, "done": 282, "cancelled": 4])
        let column = PlanColumn(plan: plan)
        let landing = try #require(column.pipeline.first { $0.stage == .landing })
        let lander = try #require(landing.agents.first)
        #expect(lander.role == .lander && lander.name == "Lander" && lander.doing == "Checking here")
        #expect(lander.tickets.map(\.task) == ["t1", "t2", "t3"], "the building lane on the train isn't landing yet")
        #expect(column.pipeline.first { $0.stage == .building }?.unattended.map(\.task) == ["t4"])
        #expect(column.facts.map(\.text) == ["Next landing: 3 tickets, 14 min", "282 of 312 done"])
    }

    @Test("a train whose CI failed puts its lander in Stuck, saying so")
    func failedTrain() throws {
        var train = PlanTrainsTests.train("integ-9", "red", lanes: ["l1"])
        train["phase"] = "ci_failed"
        train["phase_since"] = Self.now - 5 * Self.minute
        let plan = try Self.plan(
            lanes: [PlanModelTests.lane("l1", "landing", cards: ["t1"], train: "integ-9")], trains: [train],
            agents: [Self.agent("agent:i", "integrator", train: "integ-9", label: "Landing six", started: 20)])
        let column = PlanColumn(plan: plan)
        let lander = try #require(column.pipeline.first?.agents.first)
        #expect(column.pipeline.map(\.stage) == [.stuck] && lander.doing == "CI failed" && lander.doingIsTrouble)
        #expect(!column.facts.contains { $0.kind == .nextLanding }, "a failed train isn't the next landing")
    }

    @Test("initiatives in motion or asking the owner are listed; the rest are one quiet line; done ones are gone")
    func initiatives() throws {
        func theme(_ name: String, _ cards: [String], _ ordinal: Int, done: Int, total: Int, next: String = "", ask: String = "", state: String = "active") -> [String: Any] {
            var t = PlanModelTests.theme(name, cards: cards, ordinal: ordinal, state: state)
            t["counts"] = ["backlog": total - done, "todo": 0, "needs_decision": 0, "in_progress": 0, "in_review": 0, "done": done, "cancelled": 0]
            t["next"] = next
            t["owner_ask"] = ask
            return t
        }
        let plan = try Self.plan(
            themes: [
                theme("Web client", ["t1"], 1, done: 20, total: 29, next: "a shared computer signs in"),
                theme("Mobile", ["t9"], 2, done: 16, total: 24, ask: "the Play service account"),
                theme("Mac look and feel", ["t8"], 3, done: 33, total: 34),
                theme("Release", [], 4, done: 4, total: 4),
                theme("Old", [], 5, done: 1, total: 2, state: "done"),
            ],
            lanes: [PlanModelTests.lane("b", "building", cards: ["t1"])])
        let column = PlanColumn(plan: plan)
        #expect(column.initiatives.map(\.name) == ["Web client", "Mobile"])
        #expect(column.initiatives[0].next == "Next: a shared computer signs in" && !column.initiatives[0].asksOwner)
        #expect(column.initiatives[1].next == "Waiting on you: the Play service account" && column.initiatives[1].asksOwner)
        #expect(column.initiatives[0].progress == "20 of 29")
        #expect(column.quietLine == "Nothing moving: Mac look and feel, 33 of 34")
        #expect(column.initiativeCount == 3)
    }

    @Test("the footer counts what waits for a builder, what hasn't started and what's done, each opening its list")
    func footer() throws {
        let plan = try Self.plan(
            lanes: [
                PlanModelTests.lane("q1", "queued", cards: ["t1", "t2"]), PlanModelTests.lane("q2", "queued", cards: ["t2", "t3"]),
            ],
            counts: ["backlog": 20, "todo": 5, "needs_decision": 0, "in_progress": 0, "in_review": 0, "done": 796, "cancelled": 0])
        let column = PlanColumn(plan: plan, statuses: ["t1": .todo, "t2": .todo, "t3": .backlog])
        #expect(column.footer.map(\.words) == ["3 tickets waiting for a builder", "22 not started", "796 done"])
        #expect(column.footer.map(\.status) == [.todo, .backlog, .done])
    }

    @Test("the runner's own plan.json reads as a column, with nothing invented")
    func fixture() throws {
        let plan = try PlanModelTests.fixture()
        let column = PlanColumn(plan: plan)
        for group in column.pipeline {
            for agent in group.agents {
                #expect(!agent.doing.isEmpty && !agent.name.isEmpty)
                #expect(plan.agents.contains { $0.id == agent.id } || agent.id.hasPrefix("train:"), "\(agent.id) is a real agent")
            }
        }
        #expect(column.facts.contains { $0.kind == .done })
    }

    @Test("clicking a Needs You row quotes it: a task by key and title, else the words")
    func quote() {
        #expect(PlanColumn.quote(key: "ov-1", title: "Site copy") == "About ov-1 (\u{201C}Site copy\u{201D}): ")
        #expect(PlanColumn.quote(key: nil, title: "Allow\nBash?") == "About \u{201C}Allow Bash?\u{201D}: ")
    }
}
