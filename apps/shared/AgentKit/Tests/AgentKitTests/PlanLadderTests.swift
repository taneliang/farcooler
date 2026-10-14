import Foundation
import Testing

@testable import AgentKit

/// The ladder a train climbs, said compactly for the phones (ov-503, design 7).
struct PlanLadderTests {
    @Test("a phase on the ladder says its word and its step of three")
    func compactOnTheLadder() {
        #expect(PlanLadder.compact(.integrating) == "Checking Here \u{00B7} step 1 of 3")
        #expect(PlanLadder.compact(.gatingLocally) == "Checking Here \u{00B7} step 1 of 3", "combining and gating are one step")
        #expect(PlanLadder.compact(.waitingForCi) == "Checking on CI \u{00B7} step 2 of 3")
        // Waiting for lanes, a failed CI and a stall are said in place of the step they stand on.
        #expect(PlanLadder.compact(.waitingForLanes) == "Waiting for Lanes \u{00B7} step 1 of 3")
        #expect(PlanLadder.compact(.waitingForMain) == "Waiting for Main's Checks to Pass \u{00B7} step 1 of 3")
        #expect(PlanLadder.compact(.ciFailed) == "CI Failed \u{00B7} step 2 of 3")
        #expect(PlanLadder.compact(.stalled) == "Stalled \u{00B7} step 1 of 3")
        #expect(PlanLadder.compact(.readyToLand) == "Ready to Land \u{00B7} step 3 of 3")
    }

    @Test("the steps are plain words, and none is a word the runner has no state for")
    func plainSteps() {
        #expect(PlanLadder.steps == ["Checking Here", "Checking on CI", "Landed"])
        let said = (PlanLadder.steps + TrainPhase.allCases.map(\.word) + TrainPhase.allCases.map(\.sentence)).joined(separator: " ").lowercased()
        for old in ["picked", "integrat", "gated", "gating"] { #expect(!said.contains(old), "\(old)") }
    }

    @Test("every step and every phase explains itself in one line, and the step it stands on says the phase's")
    func explanations() {
        #expect(PlanLadder.explanations.count == PlanLadder.steps.count)
        for text in PlanLadder.explanations + TrainPhase.allCases.map(\.explanation) {
            #expect(!text.isEmpty && !text.contains("\n") && text.hasSuffix("."), Comment(rawValue: text))
        }
        #expect(PlanLadder.explanation(1, current: 1, phase: .ciFailed) == TrainPhase.ciFailed.explanation)
        #expect(PlanLadder.explanation(0, current: 1, phase: .ciFailed) == PlanLadder.explanations[0])
        #expect(PlanLadder.explanations[0] == "The train's full test run, on the runner.")
    }

    @Test("a train past the ladder says the phase alone")
    func compactPastTheLadder() {
        #expect(PlanLadder.compact(.landed) == "Landed")
        #expect(PlanLadder.compact(.dropped) == "Dropped")
    }

    @Test("every phase has a step the ladder holds or sits past it")
    func everyPhaseHasAStep() {
        for phase in TrainPhase.allCases {
            #expect((0...PlanLadder.steps.count).contains(PlanLadder.step(phase)), "\(phase)")
        }
    }
}

/// The train card's facts line, now AgentKit's so the Mac and the phones say one thing.
struct PlanTrainFactsTests {
    @Test("a pushed train says its commit, CI and how long it has waited")
    func factsLine() throws {
        let plan = try PlanSectionsTests.decode(
            """
            {"now_ms": 7200000, "themes": [], "lanes": [], "order": [], "cards": [], "trains": [
              {"id": "t", "short": "t", "name": "integ-9", "base": "main", "pushed_sha": "3f78754fabc", "state": "pushed", "state_since": 6480000,
               "actor": "m", "created_at": 1, "landed_at": null, "lanes": [], "ci_subject": "",
               "agent": {"agent_id": "a", "harness": "claude", "model": "opus", "started_at": 1, "ended_at": null,
                 "spend": {"input_tokens": 1000000, "output_tokens": 400000, "cache_read_tokens": 0, "cache_write_tokens": 0,
                           "cost_micros": null, "runs": 1, "unmeasured_agents": 0, "shared_agents": 0}}}]}
            """)
        let card = try #require(PlanSections(plan: plan).trains.first)
        #expect(card.facts(ci: nil, now: plan.nowMs) == "3f78754f \u{00B7} CI not read yet \u{00B7} 12 min \u{00B7} Integrator \u{00B7} Opus \u{00B7} 1.4M")
    }
}

/// The second lines the phones and the Mac draw under a lane and an agent.
struct PlanSecondLineTests {
    static func sections(_ name: String) throws -> PlanSections {
        let all = try PlanSectionsCasesTests.cases()
        let found = try #require(all.first { $0["name"] as? String == name })
        return try PlanSectionsCasesTests.Case(found).sections
    }

    @Test("a lane on a train says how it stands to the train, in place of its own state")
    func laneOnATrain() throws {
        let sections = try Self.sections("board-new-runner")
        let row = try #require(sections.trains.lazy.flatMap(\.lanes).first { $0.standing.word != PlanWords.status($0.lane) })
        let said = PlanWords.laneSecond(row.lane, agent: nil, landedTrain: nil, now: sections.now, state: row.standing.word)
        #expect(said.hasPrefix(row.standing.word), "\(said)")
        #expect(!said.hasPrefix(PlanWords.status(row.lane)), "\(said)")
    }

    @Test("a lane that still names a landed train says so, once")
    func landedTrainNamedOnce() throws {
        let sections = try Self.sections("board-old-runner")
        let row = try #require(sections.onLandedTrain.first)
        let train = try #require(row.landedTrain)
        let said = PlanWords.laneSecond(row.lane, agent: row.agent, landedTrain: train, now: sections.now)
        #expect(said.components(separatedBy: train).count - 1 <= 1, "\(said)")
    }

    @Test("an agent without a lane says what it was told and that it is not on one")
    func agentWithoutALane() throws {
        let sections = try Self.sections("board-new-runner")
        let agents = sections.fleet.agents
        let loose = try #require(agents.first { $0.lane == nil && $0.role != .orchestrator })
        let said = loose.second(now: sections.now)
        #expect(said.contains(PlanWords.notOnALane) || said.contains(loose.worksFor), Comment(rawValue: said))
    }
}
