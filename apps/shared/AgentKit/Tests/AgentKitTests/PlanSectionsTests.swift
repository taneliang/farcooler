import Foundation
import Testing

@testable import AgentKit

/// What the cases file can't say in one line (ov-503): that an older answer
/// still reads, that the phase falls back to `state`, that the strip carries
/// the band, and what `test/fixtures/plan.json` (the runner's own bytes) makes
/// of the new fields. Which ticket is in which section is the Rust core's
/// (`crates/client/src/plan_view`, ov-1016); these read its view through the
/// bridge, as the apps do.
struct PlanSectionsTests {
    static func decode(_ json: String) throws -> PlanModel { try PlanModel.decode(Data(json.utf8)) }

    @Test("older answers retain historical rows in the model without presenting them")
    func olderAnswer() throws {
        let bare = try Self.decode(#"{"now_ms": 1, "themes": [], "lines": [], "cards": []}"#)
        #expect(bare.landings.isEmpty && bare.agents.isEmpty && bare.offTheLine.isEmpty && bare.agentsSpend == nil)
        let plan = try PlanModelTests.plan(
            themes: [], lines: [], landings: [PlanModelTests.landing(9, "pushed", sha: "abc", since: 5)])
        #expect(plan.landings.count == 1)
        #expect(try #require(WirePlanView.build(plan: plan)).landings.isEmpty)
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

    @Test("a facts line keeps each span whole, so a line breaks between spans only (ov-515)")
    func factsKeepSpansTogether() {
        let line = "3f78754f \u{00B7} Running \u{00B7} 2 of 5 jobs done \u{00B7} 12 min \u{00B7} Lander \u{00B7} opus \u{00B7} 1.4M"
        let kept = PlanWords.unbroken(line)
        // The separators still break; nothing else does.
        #expect(kept.components(separatedBy: " \u{00B7} ") == ["3f78754f", "Running", "2\u{00A0}of\u{00A0}5\u{00A0}jobs\u{00A0}done", "12\u{00A0}min", "Lander", "opus", "1.4M"])
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
        #expect(landing.phase == .ciFailed)
        #expect(try #require(WirePlanView.build(plan: plan)).landings.isEmpty)
        #expect(landing.phaseSince == PlanModelTests.now - 1_200_000, "the runner's clock, twenty minutes before now")
    }

    @Test("the runner's own bytes carry the agents that exist, the orchestrator first, and each ticket's agent joins by id")
    func theRunnersAgents() throws {
        let plan = try PlanModelTests.fixture()
        #expect(plan.agents.map(\.role) == [.orchestrator, .reviewer, .builder])
        let view = try #require(WirePlanView.build(plan: plan))
        let rows = view.agents.rows
        #expect(rows.map(\.who) == ["Orchestrator \u{00B7} opus", "Reviewer \u{00B7} sonnet", "Builder \u{00B7} gpt-5.6"])
        #expect(rows.map(\.worksFor) == ["Plan", "ov-1", "ov-4"])
        #expect(rows.map(\.working) == [true, true, false] && view.section(.agents)?.count == 3)
        // The total is the runner's: the day's spend of every agent, those no longer listed too.
        let listed = plan.agents.reduce(0) { $0 + $1.spend.totalTokens }
        #expect(listed == 2_300_000)
        #expect(view.agents.header.contains("3M tokens") && view.agents.header.contains("2 working"), "the header counts agents that are gone")
        // The reviewer is the one that works ov-1, for the row under its title.
        let review = try #require(view.inReview.first)
        #expect(review.key == "ov-1" && review.agent?.id == plan.agents[1].id)
        #expect(view.landings.isEmpty)
        #expect(view.waiting(.next).map(\.key) == ["ov-2"])
    }
}
