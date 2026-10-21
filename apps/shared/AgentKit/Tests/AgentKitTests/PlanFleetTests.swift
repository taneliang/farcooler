import Foundation
import Testing

@testable import AgentKit

// The Agents list holds the agents that still exist (ov-518): the runner
// leaves out a finished subagent and an exited pane, so the fleet has no
// Finished group and the header's total comes from the runner's, which still
// counts what the gone ones spent.
@Suite("The Plan's agents list")
struct PlanFleetTests {
    private static let now: Int64 = 1_800_000_000_000
    private static let en = Locale(identifier: "en_US")

    private static func agent(
        _ id: String, role: String = "subagent", working: Bool, startedAgo: Int64, lastAgo: Int64, tokens: UInt64
    ) throws -> PlanFleetAgent {
        let json = """
            {"id":"\(id)","role":"\(role)","harness":"claude","model":"sonnet","label":"","lane":null,"train":null,
             "card":null,"terminal":null,"working":\(working),"started_at":\(now - startedAgo),"last_at":\(now - lastAgo),
             "spend":{"input_tokens":\(tokens),"cost_micros":null}}
            """
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(PlanFleetAgent.self, from: Data(json.utf8))
    }

    @Test("a subagent resumed for another round reads Working with how long it has run, an idle pane reads Idle")
    func states() throws {
        let hour: Int64 = 3_600_000
        let resumed = try Self.agent("agent:a", working: true, startedAgo: 3 * hour, lastAgo: 60_000, tokens: 10)
        let idle = try Self.agent("pane:p", role: "agent", working: false, startedAgo: 5 * hour, lastAgo: 3 * hour, tokens: 10)
        #expect(resumed.state(now: Self.now) == "Working \u{00B7} 3 h")
        #expect(idle.state(now: Self.now) == "Idle \u{00B7} active 3 h ago")
        #expect(!idle.state(now: Self.now).contains("Done"))
        let unseen = try Self.agent("pane:o", role: "orchestrator", working: false, startedAgo: Self.now, lastAgo: Self.now, tokens: 0)
        #expect(unseen.lastAt == 0 && unseen.state(now: Self.now) == "Idle", "no turn in the window: no invented age")
    }

    @Test("the list is exactly the agents the runner sent, working ones first however little they spent")
    func listsWhatExists() throws {
        let idle = try Self.agent("pane:big", role: "agent", working: false, startedAgo: 9_000_000, lastAgo: 7_000_000, tokens: 5_000_000)
        let small = try Self.agent("agent:small", working: true, startedAgo: 600_000, lastAgo: 1_000, tokens: 1_000)
        let fleet = PlanFleet([idle, small])
        #expect(fleet.agents.map(\.id) == ["agent:small", "pane:big"])
        #expect(fleet.workingCount == 1)
        #expect(!fleet.isEmpty)
        #expect(PlanFleet([]).header(locale: Self.en) == PlanWords.noAgents)
        // With none alive the day's spend still shows, in the live header's format.
        var spent = PlanSpend()
        spent.inputTokens = 1_200_000
        spent.costMicros = 12_000_000
        #expect(PlanFleet([], spend: spent).header(locale: Self.en) == "No agents running \u{00B7} 1.2M tokens in the last 24 hours \u{00B7} \u{2248}$12.00")
    }

    @Test("the header's total is the runner's, which counts agents that are gone; without it the listed agents' sum stands in")
    func headerTotal() throws {
        let live = try Self.agent("agent:live", working: true, startedAgo: 600_000, lastAgo: 1_000, tokens: 1_000_000)
        var gone = PlanSpend()
        gone.inputTokens = 4_000_000
        gone.costMicros = 12_000_000
        let counted = PlanFleet([live], spend: gone).header(locale: Self.en)
        #expect(counted == "1 agent \u{00B7} 1 working \u{00B7} 4M tokens in the last 24 hours \u{00B7} \u{2248}$12.00", Comment(rawValue: counted))
        let summed = PlanFleet([live]).header(locale: Self.en)
        #expect(summed.hasPrefix("1 agent \u{00B7} 1 working \u{00B7} 1M tokens in the last 24 hours"), Comment(rawValue: summed))
        #expect(!summed.contains("$"), "an unpriced agent leaves no dollar total")
    }

    @Test("the list's count and the Agents card's differ by the orchestrator, and the header says so (ov-660)")
    func countsAgreeOrSayWhy() throws {
        // The card's fleet: the orchestrator with two subagents, a builder with one.
        let card = RunningAgents.workspace(
            RunningAgentsTests.main, runner: RunningAgentsTests.runner, fleet: RunningAgentsTests.fleet)
        #expect(card.count == 4)
        // The same agents as the plan lists them: every one that exists, the orchestrator too.
        let listed = try PlanFleet([
            Self.agent("pane:orch", role: "orchestrator", working: true, startedAgo: 600_000, lastAgo: 1_000, tokens: 10),
            Self.agent("agent:s1", working: true, startedAgo: 600_000, lastAgo: 1_000, tokens: 10),
            Self.agent("agent:s2", working: true, startedAgo: 600_000, lastAgo: 1_000, tokens: 10),
            Self.agent("pane:builder", role: "build", working: false, startedAgo: 600_000, lastAgo: 100_000, tokens: 10),
            Self.agent("agent:s3", working: false, startedAgo: 600_000, lastAgo: 100_000, tokens: 10),
        ])
        #expect(listed.includesOrchestrator)
        #expect(listed.agents.count == card.count + 1, "the plan counts the orchestrator; the card is what it is running")
        #expect(listed.countWords == "5 agents, including the orchestrator")
        #expect(listed.header(locale: Self.en).hasPrefix("5 agents, including the orchestrator \u{00B7} 3 working"))
        // Without the orchestrator in the list the two agree, and nothing is explained that isn't there.
        let without = PlanFleet(listed.agents.filter { $0.role != .orchestrator })
        #expect(!without.includesOrchestrator)
        #expect(without.agents.count == card.count)
        #expect(without.countWords == "4 agents")
        #expect(!without.header(locale: Self.en).contains("orchestrator"))
        #expect(try PlanFleet([Self.agent("pane:o", role: "orchestrator", working: true, startedAgo: 1, lastAgo: 1, tokens: 1)]).countWords == "1 agent, including the orchestrator")
    }

    @Test("agents_spend decodes, and a runner without it leaves nil")
    func decodesTheTotal() throws {
        let base = """
            {"now_ms":1,"themes":[],"lanes":[],"order":[],"cards":[]
            """
        let with = base + #","agents_spend":{"input_tokens":7,"cost_micros":9}}"#
        #expect(try PlanModel.decode(Data(with.utf8)).agentsSpend?.totalTokens == 7)
        #expect(try PlanModel.decode(Data((base + "}").utf8)).agentsSpend == nil)
    }
}
