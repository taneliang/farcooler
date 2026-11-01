import Foundation
import Testing

@testable import AgentKit

/// What the cases file can't say in one line (ov-503): that an older answer
/// still reads, that the phase falls back to `state`, that the strip carries
/// the band, and what `test/fixtures/plan.json` (the runner's own bytes) makes
/// of the new fields.
struct PlanSectionsTests {
    static func decode(_ json: String) throws -> PlanModel { try PlanModel.decode(Data(json.utf8)) }

    @Test("an answer with no phase, landings or agents reads, and a landing falls back to its state")
    func olderAnswer() throws {
        let bare = try Self.decode(#"{"now_ms": 1, "themes": [], "lines": [], "cards": []}"#)
        #expect(bare.landings.isEmpty && bare.agents.isEmpty && bare.offTheLine.isEmpty && bare.agentsSpend == nil)
        let plan = try PlanModelTests.plan(
            themes: [], lines: [], landings: [PlanModelTests.landing(9, "pushed", sha: "abc", since: 5)])
        let landing = try #require(plan.landings.first)
        #expect(landing.phase == nil && landing.phaseSince == nil)
        #expect(landing.phase(in: plan) == .waitingForCi && landing.phaseSince(in: plan) == 5)
        #expect(plan.agents.isEmpty)
        #expect(PlanSections(plan: plan).landings.map(\.phase) == [.waitingForCi])
    }

    @Test("a landing carrying a field this build doesn't read (an older runner's held_by) reads, and no line is drawn from it")
    func extraFieldsAreIgnored() throws {
        var landing = PlanModelTests.landing(9, "open", phase: "waiting_for_tickets", since: 5, phaseSince: 9)
        landing["held_by"] = [["ticket": "l1", "name": "slow"]]
        let plan = try PlanModelTests.plan(themes: [], lines: [], landings: [landing])
        let card = try #require(PlanSections(plan: plan).landings.first)
        #expect(card.phase == .waitingForTickets)
        #expect(card.spoken(ci: nil, now: plan.nowMs).contains("slow") == false, "nothing is said of a hold")
    }

    @Test("a phase this build has no word for reads the state, and the runner's phase wins when it has one")
    func unknownPhase() throws {
        func plan(_ phase: String?) throws -> PlanModel {
            try PlanModelTests.plan(
                themes: [], lines: [], landings: [PlanModelTests.landing(9, "red", phase: phase, since: 5, phaseSince: 9)])
        }
        let unknown = try plan("from_the_future")
        #expect(unknown.landings[0].phase == .unknown)
        #expect(unknown.landings[0].phase(in: unknown) == .ciFailed && unknown.landings[0].phaseSince(in: unknown) == 5)
        let stalled = try plan("stalled")
        #expect(stalled.landings[0].phase(in: stalled) == .stalled && stalled.landings[0].phaseSince(in: stalled) == 9)
        let null = try plan(nil)
        #expect(null.landings[0].phase == nil && null.landings[0].phase(in: null) == .ciFailed)
    }

    @Test("an open landing waits for tickets while one isn't ready, and checks here once all are")
    func openFallback() throws {
        func plan(_ stage: String, fixing: Bool = false) throws -> PlanModel {
            try PlanModelTests.plan(
                themes: [],
                lines: [
                    PlanModelTests.line("t1", "landing", landing: 9), PlanModelTests.line("t2", stage, landing: 9, fixing: fixing),
                ],
                landings: [PlanModelTests.landing(9, "open", tickets: ["t1", "t2"])])
        }
        for (stage, fixing) in [("building", true), ("building", false), ("reviewing", false), ("needs_you", false)] {
            let waiting = try plan(stage, fixing: fixing)
            #expect(waiting.landings[0].phase(in: waiting) == .waitingForTickets, "\(stage) fixing: \(fixing)")
        }
        let ready = try plan("landed")
        #expect(ready.landings[0].phase(in: ready) == .integrating, "a landed ticket is not a hold")
        let allAboard = try plan("landing")
        #expect(allAboard.landings[0].phase(in: allAboard) == .integrating)
    }

    /// A waiting ticket, one in review and one building, each naming a landing that has landed.
    static func landedLandingPlan() throws -> PlanModel {
        try PlanModelTests.plan(
            themes: [],
            lines: [
                PlanModelTests.line("t1", "waiting", landing: 85), PlanModelTests.line("t2", "reviewing", landing: 83),
                PlanModelTests.line("t3", "building", landing: 82), PlanModelTests.line("t4", "waiting", rank: 1),
            ],
            landings: [
                PlanModelTests.landing(85, "landed", tickets: ["t1"]), PlanModelTests.landing(83, "landed", tickets: ["t2"]),
                PlanModelTests.landing(82, "landed", tickets: ["t3"]),
            ])
    }

    @Test("a waiting ticket naming a landed landing is in the folded section; one in review or building routes by its stage and says it names it")
    func landedLandingFolds() throws {
        let sections = PlanSections(plan: try Self.landedLandingPlan())
        #expect(PlanSections.Kind.onLandedLanding.startsFolded)
        #expect(PlanSections.Kind.allCases.filter(\.startsFolded) == [.onLandedLanding], "only this section starts folded")
        #expect(sections.onLandedLanding.map(\.line.task) == ["t1"] && sections.count(.onLandedLanding) == 1)
        #expect(sections.onLandedLanding.first?.landedLanding == "Landing 85")
        #expect(sections.inReview.map(\.line.task) == ["t2"] && sections.inReview.first?.landedLanding == "Landing 83")
        #expect(sections.building.map(\.line.task) == ["t3"] && sections.building.first?.landedLanding == "Landing 82")
        #expect(sections.waiting.next.map(\.line.task) == ["t4"] && sections.waiting.next.first?.landedLanding == nil)
        #expect(sections.shown.contains(.onLandedLanding))
    }

    @Test("a ticket in a fix round is In Review, and building otherwise; a fix round after a review keeps it where the review was")
    func fixRoundsAreInReview() throws {
        let plan = try PlanModelTests.plan(
            themes: [],
            lines: [
                PlanModelTests.line("t1", "building"), PlanModelTests.line("t2", "building", fixRounds: 1, fixing: true),
                PlanModelTests.line("t3", "reviewing"),
            ])
        let sections = PlanSections(plan: plan)
        #expect(sections.building.map(\.line.task) == ["t1"])
        #expect(sections.inReview.map(\.line.task) == ["t2", "t3"])
        #expect(PlanGlanceSentence(sections: sections).now == "1 building \u{00B7} 2 in review")
    }

    @Test("the strip carries the band, and its old parts are unchanged")
    func stripBand() throws {
        let plan = try PlanStripTests.plan()
        let strip = PlanStrip(plan: plan, needsYou: 2, orchestrator: .working)
        // t1 builds; t2 is in review and t3 in a fix round, which is In Review still.
        #expect(strip.band?.now == "1 building \u{00B7} 2 in review")
        #expect(strip.band?.next == "Plan phones, then Lint rules")
        #expect(strip.text == "2 need you · Visual polish Building · Phone strip In Review · +1 · next: Plan phones")
    }

    @Test("the Now line calls a landing by its number and phase, never its title (ov-514)")
    func nowLineDropsTheTitle() throws {
        let title = "Landing 89 \u{00B7} The agents table and the themes grid on the phones"
        let plan = try PlanModelTests.plan(
            themes: [], lines: [],
            landings: [
                PlanModelTests.landing(89, "pushed", sha: "abc", phase: "waiting_for_ci", since: 5, title: title),
                PlanModelTests.landing(90, "pushed", sha: "abd", phase: "waiting_for_ci", since: 6, title: "Another long title"),
            ])
        #expect(plan.landings[0].subtitle == "The agents table and the themes grid on the phones", "the runner's \"Landing 89 \u{00B7}\" is not said twice")
        #expect(plan.landings[0].label == "Landing 89")
        let now = try #require(PlanGlanceSentence(sections: PlanSections(plan: plan)).now)
        #expect(now == "Landing 89 checking on CI \u{00B7} 1 more landing")
        #expect(!now.contains("agents table"))
        // Short enough for 320 pt at the default size and for a watch's line
        // ("Landing" is two letters longer than "Train" was).
        #expect(now.count <= 42)
    }

    @Test("a facts line keeps each span whole, so a line breaks between spans only (ov-515)")
    func factsKeepSpansTogether() {
        let line = "3f78754f \u{00B7} Running \u{00B7} 2 of 5 jobs done \u{00B7} 12 min \u{00B7} Lander \u{00B7} Opus \u{00B7} 1.4M"
        let kept = PlanSections.LandingCard.unbroken(line)
        // The separators still break; nothing else does.
        #expect(kept.components(separatedBy: " \u{00B7} ") == ["3f78754f", "Running", "2\u{00A0}of\u{00A0}5\u{00A0}jobs\u{00A0}done", "12\u{00A0}min", "Lander", "Opus", "1.4M"])
        #expect(kept.replacingOccurrences(of: "\u{00A0}", with: " ") == line)
        #expect(!kept.contains("12 min"))
        // The band's Now line the same: "2 building" stays whole.
        let now = PlanGlanceSentence(now: "Landing 86 waiting for CI \u{00B7} 2 more landings \u{00B7} 2 building \u{00B7} 3 in review")
        #expect(now.nowLineUnbroken.contains("2\u{00A0}building") && now.nowLineUnbroken.contains("Landing\u{00A0}86\u{00A0}waiting\u{00A0}for\u{00A0}CI"))
        #expect(now.nowLineUnbroken.replacingOccurrences(of: "\u{00A0}", with: " ") == now.nowLine)
    }

    @Test("the runner's own bytes carry the phase on a red landing")
    func theRunnersFixture() throws {
        let plan = try PlanModelTests.fixture()
        let landing = try #require(plan.landings.first)
        #expect(landing.phase == .ciFailed && landing.phase(in: plan) == .ciFailed)
        #expect(landing.phaseSince == PlanModelTests.now - 1_200_000, "the runner's clock, twenty minutes before now")
        #expect(PlanSections(plan: plan).landings.first?.phase == .ciFailed)
    }

    @Test("the runner's own bytes carry the agents that exist, the orchestrator first, and each ticket's agent joins by id")
    func theRunnersAgents() throws {
        let plan = try PlanModelTests.fixture()
        #expect(plan.agents.map(\.role) == [.orchestrator, .reviewer, .builder])
        let sections = PlanSections(plan: plan)
        let fleet = sections.fleet
        #expect(fleet.agents.map(\.who) == ["Orchestrator \u{00B7} Opus", "Reviewer \u{00B7} Sonnet", "Builder \u{00B7} gpt-5.6"])
        #expect(fleet.agents.map(\.worksFor) == ["Plan", "ov-1", "ov-4"])
        #expect(fleet.agents[1].title == "Review ov-1" && fleet.agents.map(\.working) == [true, true, false])
        #expect(sections.count(.agents) == 3)
        // The total is the runner's: the day's spend of every agent, those no longer listed too.
        let listed = fleet.agents.reduce(0) { $0 + $1.spend.totalTokens }
        #expect(listed == 2_300_000)
        #expect(fleet.totalTokens == 3_000_000 && fleet.totalCostMicros == 20_000_000, "the header counts agents that are gone")
        #expect(fleet.header(locale: Locale(identifier: "en_US")).contains("2 working"))
        // The reviewer is the one that works ov-1, for the row under its title.
        let review = try #require(sections.inReview.first)
        #expect(review.line.key == "ov-1" && review.agent?.role == .reviewer)
        // ov-4 is on Landing 9, whose lander the agents list no longer has.
        let card = try #require(sections.landings.first)
        #expect(card.tickets.map(\.key) == ["ov-4"] && card.lander == nil)
        #expect(sections.waiting.next.map(\.line.key) == ["ov-2"])
    }
}
