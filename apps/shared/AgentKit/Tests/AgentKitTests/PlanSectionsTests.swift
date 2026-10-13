import Foundation
import Testing

@testable import AgentKit

/// What the cases file can't say in one line (ov-503): that an older answer
/// still reads, that the phase falls back to `state`, that the strip carries
/// the band, and what `test/fixtures/plan.json` (the runner's own bytes) makes
/// of the new fields.
struct PlanSectionsTests {
    static func decode(_ json: String) throws -> PlanModel { try PlanModel.decode(Data(json.utf8)) }

    @Test("an answer with no phase, held_by or agents reads, and a train falls back to its state")
    func olderAnswer() throws {
        let plan = try Self.decode(
            """
            {"now_ms": 1, "themes": [], "lanes": [], "order": [], "cards": [], "trains": [
              {"id": "t", "short": "t", "name": "integ-9", "base": "main", "pushed_sha": "abc", "state": "pushed", "state_since": 5,
               "actor": "m", "created_at": 1, "landed_at": null, "lanes": [], "ci_subject": ""}]}
            """)
        let train = try #require(plan.trains.first)
        #expect(train.phase == nil && train.phaseSince == nil && train.holders.isEmpty)
        #expect(train.phase(in: plan) == .waitingForCi && train.phaseSince(in: plan) == 5)
        #expect(plan.agents.isEmpty)
        #expect(PlanSections(plan: plan).shipping.map(\.phase) == [.waitingForCi])
    }

    @Test("a phase this build has no word for reads the state, and the runner's phase wins when it has one")
    func unknownPhase() throws {
        func plan(_ phase: String) throws -> PlanModel {
            try Self.decode(
                """
                {"now_ms": 1, "themes": [], "lanes": [], "order": [], "cards": [], "trains": [
                  {"id": "t", "short": "t", "name": "integ-9", "base": "main", "pushed_sha": null, "state": "red", "state_since": 5,
                   "phase": \(phase), "phase_since": 9, "held_by": [], "actor": "m", "created_at": 1, "landed_at": null,
                   "lanes": [], "ci_subject": ""}]}
                """)
        }
        let unknown = try plan("\"from_the_future\"")
        #expect(unknown.trains[0].phase == .unknown)
        #expect(unknown.trains[0].phase(in: unknown) == .ciFailed && unknown.trains[0].phaseSince(in: unknown) == 5)
        let stalled = try plan("\"stalled\"")
        #expect(stalled.trains[0].phase(in: stalled) == .stalled && stalled.trains[0].phaseSince(in: stalled) == 9)
        let null = try plan("null")
        #expect(null.trains[0].phase == nil && null.trains[0].phase(in: null) == .ciFailed)
    }

    @Test("an integrating train waits for lanes while one isn't ready, and integrates once all are")
    func integratingFallback() throws {
        func plan(_ state: String) throws -> PlanModel {
            try PlanTrainsTests.plan(
                trains: [PlanTrainsTests.train("integ-9", "integrating", lanes: ["a", "b"])],
                lanes: [PlanModelTests.lane("a", "landing", cards: []), PlanModelTests.lane("b", state, cards: [])])
        }
        let waiting = try plan("fixing")
        #expect(waiting.trains[0].phase(in: waiting) == .waitingForLanes)
        let ready = try plan("landed")
        #expect(ready.trains[0].phase(in: ready) == .integrating)
    }

    @Test("the strip carries the band, and its old parts are unchanged")
    func stripBand() throws {
        let plan = try PlanStripTests.plan()
        let strip = PlanStrip(plan: plan, needsYou: 2, orchestrator: .working)
        #expect(strip.band?.now == "1 building · 2 in review")
        #expect(strip.band?.next == "plan-phones, then vis-lint")
        #expect(strip.text == "2 need you · mac-vis Building · phones-b In Review · +1 · next: plan-phones")
    }

    @Test("the runner's own bytes carry the phase on a red train")
    func theRunnersFixture() throws {
        let plan = try PlanModelTests.fixture()
        let train = try #require(plan.trains.first)
        #expect(train.phase == .ciFailed && train.phase(in: plan) == .ciFailed)
        #expect(train.phaseSince != nil && train.holders.isEmpty)
        #expect(PlanSections(plan: plan).shipping.first?.phase == .ciFailed)
    }

    @Test("the runner's own bytes carry the agents, the orchestrator first, a subagent nobody recorded unjoined")
    func theRunnersAgents() throws {
        let plan = try PlanModelTests.fixture()
        #expect(plan.agents.map(\.role) == [.orchestrator, .review, .subagent])
        let fleet = PlanSections(plan: plan).fleet
        #expect(fleet.working.map(\.who) == ["Orchestrator · Opus"] && fleet.finished.map(\.worksFor) == ["mac-ux", PlanWords.notOnALane])
        #expect(fleet.finished[1].label == "Design ov-503" && fleet.working[0].worksFor == "Plan")
        #expect(fleet.totalTokens == 2_390_000 && fleet.totalCostMicros == nil, "a hole in the prices leaves no dollar total")
        // The review agent is the one that works mac-ux, for the row under its title.
        let mac = try #require(PlanSections(plan: plan).shipping.first?.lanes.first)
        #expect(mac.name == "mac-ux")
        #expect(PlanSections(plan: plan).fleet.finished.first?.lane?.name == "mac-ux")
    }
}
