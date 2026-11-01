import Foundation
import Testing

@testable import AgentKit

/// The ladder a landing climbs, said compactly for the phones (ov-503, design 7).
struct PlanLadderTests {
    @Test("a phase on the ladder says its word and its step of three")
    func compactOnTheLadder() {
        #expect(PlanLadder.compact(.integrating) == "Checking Here \u{00B7} step 1 of 3")
        #expect(PlanLadder.compact(.gatingLocally) == "Checking Here \u{00B7} step 1 of 3", "combining and gating are one step")
        #expect(PlanLadder.compact(.waitingForCi) == "Checking on CI \u{00B7} step 2 of 3")
        // Waiting for tickets, a failed CI and a stall are said in place of the step they stand on.
        #expect(PlanLadder.compact(.waitingForTickets) == "Waiting for Tickets \u{00B7} step 1 of 3")
        #expect(PlanLadder.compact(.waitingForPreviousLanding) == "Waiting for the Earlier Landing \u{00B7} step 1 of 3")
        #expect(PlanLadder.compact(.waitingForMain) == "Waiting for Main's Checks to Pass \u{00B7} step 1 of 3")
        #expect(PlanLadder.compact(.ciFailed) == "CI Failed \u{00B7} step 2 of 3")
        #expect(PlanLadder.compact(.stalled) == "Stalled \u{00B7} step 1 of 3")
        #expect(PlanLadder.compact(.readyToLand) == "Ready to Land \u{00B7} step 3 of 3")
    }

    @Test("the steps are plain words, and none is a word the runner has no state for")
    func plainSteps() {
        #expect(PlanLadder.steps == ["Checking Here", "Checking on CI", "Landed"])
        let said = (PlanLadder.steps + LandingPhase.allCases.map(\.word) + LandingPhase.allCases.map(\.sentence)).joined(separator: " ").lowercased()
        for old in ["picked", "integrat", "gated", "gating"] { #expect(!said.contains(old), "\(old)") }
    }

    @Test("every step and every phase explains itself in one line, and the step it stands on says the phase's")
    func explanations() {
        #expect(PlanLadder.explanations.count == PlanLadder.steps.count)
        for text in PlanLadder.explanations + LandingPhase.allCases.map(\.explanation) {
            #expect(!text.isEmpty && !text.contains("\n") && text.hasSuffix("."), Comment(rawValue: text))
        }
        #expect(PlanLadder.explanation(1, current: 1, phase: .ciFailed) == LandingPhase.ciFailed.explanation)
        #expect(PlanLadder.explanation(0, current: 1, phase: .ciFailed) == PlanLadder.explanations[0])
        #expect(PlanLadder.explanations[0] == "The full test run, on the runner.")
    }

    @Test("a landing past the ladder says the phase alone")
    func compactPastTheLadder() {
        #expect(PlanLadder.compact(.landed) == "Landed")
        #expect(PlanLadder.compact(.dropped) == "Dropped")
    }

    @Test("every phase has a step the ladder holds or sits past it")
    func everyPhaseHasAStep() {
        for phase in LandingPhase.allCases {
            #expect((0...PlanLadder.steps.count).contains(PlanLadder.step(phase)), "\(phase)")
        }
    }
}

/// The landing card's facts line, now AgentKit's so the Mac and the phones say one thing.
struct PlanLandingFactsTests {
    @Test("a pushed landing says its commit, CI, how long it has waited and who lands it")
    func factsLine() throws {
        var lander = PlanModelTests.agent("lander-1", role: "lander", model: "opus")
        lander["spend"] = ["input_tokens": 1_000_000, "output_tokens": 400_000]
        let plan = try PlanModelTests.plan(
            themes: [], lines: [], landings: [PlanModelTests.landing(9, "pushed", sha: "3f78754fabc", since: 6_480_000, lander: "lander-1")],
            agents: [lander])
        let card = try #require(PlanSections(plan: planAt(plan, 7_200_000)).landings.first)
        #expect(card.facts(ci: nil, now: 7_200_000) == "3f78754f \u{00B7} CI not read yet \u{00B7} 12 min \u{00B7} Lander \u{00B7} opus \u{00B7} 1.4M")
    }

    private func planAt(_ plan: PlanModel, _ now: Int64) -> PlanModel {
        var plan = plan
        plan.nowMs = now
        return plan
    }
}

/// The second lines the phones and the Mac draw under a ticket and an agent.
struct PlanSecondLineTests {
    static func sections(_ name: String) throws -> PlanSections {
        let all = try PlanSectionsCasesTests.cases()
        let found = try #require(all.first { $0["name"] as? String == name })
        return try PlanSectionsCasesTests.Case(found).sections
    }

    @Test("a ticket on a landing says how it stands to the landing, in place of its own stage")
    func ticketOnALanding() throws {
        let sections = try Self.sections("board-with-phases")
        let row = try #require(sections.landings.lazy.flatMap(\.tickets).first { $0.standing.word != PlanWords.status($0.line) })
        let said = PlanWords.ticketSecond(row.line, agent: nil, landedLanding: nil, now: sections.now, state: row.standing.word)
        #expect(said.hasPrefix(row.standing.word), "\(said)")
        #expect(!said.hasPrefix(PlanWords.status(row.line)), "\(said)")
    }

    @Test("a ticket that still names a landed landing says so, once")
    func landedLandingNamedOnce() throws {
        let sections = PlanSections(plan: try PlanSectionsTests.landedLandingPlan())
        let row = try #require(sections.onLandedLanding.first)
        let landing = try #require(row.landedLanding)
        let said = PlanWords.ticketSecond(row.line, agent: row.agent, landedLanding: landing, now: sections.now)
        #expect(said.components(separatedBy: landing).count - 1 <= 1, "\(said)")
    }

    @Test("an agent on no ticket says what it was told and that it is not on one")
    func agentOnNoTicket() throws {
        let sections = try Self.sections("agents-exist-only")
        let agents = sections.fleet.agents
        let loose = try #require(agents.first { $0.tickets.isEmpty && $0.role != .orchestrator })
        let said = loose.second(now: sections.now)
        #expect(said.contains(PlanWords.onNoTicket), Comment(rawValue: said))
    }
}
