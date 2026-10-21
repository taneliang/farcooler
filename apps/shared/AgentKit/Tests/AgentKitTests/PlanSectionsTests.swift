import Foundation
import Testing

@testable import AgentKit

/// What the cases file can't say in one line (ov-503): that an older answer
/// still reads, that the phase falls back to `state`, that the strip carries
/// the band, and what `test/fixtures/plan.json` (the runner's own bytes) makes
/// of the new fields.
struct PlanSectionsTests {
    static func decode(_ json: String) throws -> PlanModel { try PlanModel.decode(Data(json.utf8)) }

    @Test("an answer with no phase or agents reads, and a train falls back to its state")
    func olderAnswer() throws {
        let plan = try Self.decode(
            """
            {"now_ms": 1, "themes": [], "lanes": [], "order": [], "cards": [], "trains": [
              {"id": "t", "short": "t", "name": "integ-9", "base": "main", "pushed_sha": "abc", "state": "pushed", "state_since": 5,
               "actor": "m", "created_at": 1, "landed_at": null, "lanes": [], "ci_subject": ""}]}
            """)
        let train = try #require(plan.trains.first)
        #expect(train.phase == nil && train.phaseSince == nil)
        #expect(train.phase(in: plan) == .waitingForCi && train.phaseSince(in: plan) == 5)
        #expect(plan.agents.isEmpty)
        #expect(PlanSections(plan: plan).trains.map(\.phase) == [.waitingForCi])
    }

    @Test("a runner before Train 119 still sends held_by: the train reads, and no line is drawn from it")
    func olderRunnerHeldBy() throws {
        let plan = try Self.decode(
            """
            {"now_ms": 7200000, "themes": [], "lanes": [], "order": [], "cards": [], "trains": [
              {"id": "t", "short": "t", "name": "integ-9", "base": "main", "pushed_sha": null, "state": "integrating", "state_since": 5,
               "phase": "waiting_for_lanes", "phase_since": 9, "held_by": [{"lane": "l1", "name": "slow"}],
               "actor": "m", "created_at": 1, "landed_at": null, "lanes": [], "ci_subject": ""}]}
            """)
        let card = try #require(PlanSections(plan: plan).trains.first)
        #expect(card.phase == .waitingForLanes)
        #expect(card.spoken(ci: nil, now: plan.nowMs).contains("slow") == false, "nothing is said of a hold")
    }

    @Test("a phase this build has no word for reads the state, and the runner's phase wins when it has one")
    func unknownPhase() throws {
        func plan(_ phase: String) throws -> PlanModel {
            try Self.decode(
                """
                {"now_ms": 1, "themes": [], "lanes": [], "order": [], "cards": [], "trains": [
                  {"id": "t", "short": "t", "name": "integ-9", "base": "main", "pushed_sha": null, "state": "red", "state_since": 5,
                   "phase": \(phase), "phase_since": 9, "actor": "m", "created_at": 1, "landed_at": null,
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

    @Test("the Now line calls a train by its number and phase, never its title (ov-514)")
    func nowLineDropsTheTitle() throws {
        let title = "Train 89 \u{00B7} The agents table and the themes grid on the phones"
        let plan = try Self.decode(
            """
            {"now_ms": 1, "themes": [], "lanes": [], "order": [], "cards": [], "trains": [
              {"id": "t", "short": "t", "name": "integ-89", "base": "main", "pushed_sha": "abc", "state": "pushed", "state_since": 5,
               "actor": "m", "created_at": 1, "landed_at": null, "lanes": [], "ci_subject": "", "title": "\(title)",
               "phase": "waiting_for_ci", "phase_since": 5},
              {"id": "u", "short": "u", "name": "integ-90", "base": "main", "pushed_sha": "abd", "state": "pushed", "state_since": 6,
               "actor": "m", "created_at": 1, "landed_at": null, "lanes": [], "ci_subject": "", "title": "Another long title",
               "phase": "waiting_for_ci", "phase_since": 6}]}
            """)
        #expect(plan.trains[0].subtitle == "The agents table and the themes grid on the phones", "the runner's \"Train 89 \u{00B7}\" is not said twice")
        #expect(plan.trains[0].label == "Train 89")
        let now = try #require(PlanGlanceSentence(sections: PlanSections(plan: plan)).now)
        #expect(now == "Train 89 checking on CI \u{00B7} 1 more landing")
        #expect(!now.contains("agents table"))
        // Short enough for 320 pt at the default size and for a watch's line.
        #expect(now.count <= 40)
    }

    @Test("a facts line keeps each span whole, so a line breaks between spans only (ov-515)")
    func factsKeepSpansTogether() {
        let line = "3f78754f \u{00B7} Running \u{00B7} 2 of 5 jobs done \u{00B7} 12 min \u{00B7} Integrator \u{00B7} Opus \u{00B7} 1.4M"
        let kept = PlanSections.TrainCard.unbroken(line)
        // The separators still break; nothing else does.
        #expect(kept.components(separatedBy: " \u{00B7} ") == ["3f78754f", "Running", "2\u{00A0}of\u{00A0}5\u{00A0}jobs\u{00A0}done", "12\u{00A0}min", "Integrator", "Opus", "1.4M"])
        #expect(kept.replacingOccurrences(of: "\u{00A0}", with: " ") == line)
        #expect(!kept.contains("12 min"))
        // The band's Now line the same: "2 building" stays whole.
        let now = PlanGlanceSentence(now: "Train 86 waiting for CI \u{00B7} 2 more landings \u{00B7} 2 building \u{00B7} 3 in review")
        #expect(now.nowLineUnbroken.contains("2\u{00A0}building") && now.nowLineUnbroken.contains("Train\u{00A0}86\u{00A0}waiting\u{00A0}for\u{00A0}CI"))
        #expect(now.nowLineUnbroken.replacingOccurrences(of: "\u{00A0}", with: " ") == now.nowLine)
    }

    @Test("the runner's own bytes carry the phase on a red train")
    func theRunnersFixture() throws {
        let plan = try PlanModelTests.fixture()
        let train = try #require(plan.trains.first)
        #expect(train.phase == .ciFailed && train.phase(in: plan) == .ciFailed)
        #expect(train.phaseSince != nil)
        #expect(PlanSections(plan: plan).trains.first?.phase == .ciFailed)
    }

    @Test("the runner's own bytes carry the agents that exist, the orchestrator first, a subagent nobody recorded unjoined")
    func theRunnersAgents() throws {
        let plan = try PlanModelTests.fixture()
        #expect(plan.agents.map(\.role) == [.orchestrator, .review, .subagent])
        let fleet = PlanSections(plan: plan).fleet
        #expect(fleet.agents.map(\.who) == ["Orchestrator \u{00B7} Opus", "Reviewer \u{00B7} Sonnet", "Subagent \u{00B7} Sonnet"])
        #expect(fleet.agents.map(\.worksFor) == ["Plan", "mac-ux", PlanWords.notOnALane])
        #expect(fleet.agents[2].label == "Design ov-503" && fleet.agents.allSatisfy(\.working))
        #expect(PlanSections(plan: plan).count(.agents) == 3)
        // The total is the runner's: the day's spend of every agent, those no longer listed too.
        let listed = fleet.agents.reduce(0) { $0 + $1.spend.totalTokens }
        #expect(listed == 2_390_000)
        #expect(fleet.totalTokens == 3_000_000 && fleet.totalCostMicros == 20_000_000, "the header counts agents that are gone")
        #expect(fleet.header(locale: Locale(identifier: "en_US")).contains("3 working"))
        // The review agent is the one that works mac-ux, for the row under its title.
        let mac = try #require(PlanSections(plan: plan).trains.first?.lanes.first)
        #expect(mac.name == "mac-ux")
        #expect(PlanSections(plan: plan).fleet.agents.first { $0.role == .review }?.lane?.name == "mac-ux")
    }
}
