import Foundation
import Testing

@testable import AgentKit

/// The landing card's facts line (ov-503): the view's commit, phase time and
/// lander words with the app's own CI read, which the view doesn't carry yet
/// (ov-1015 phase 2). The ladder and every other word are the Rust core's
/// (ov-1016).
struct PlanLandingFactsTests {
    @Test("a pushed landing says its commit, CI, how long it has waited and who lands it")
    func factsLine() throws {
        var lander = PlanModelTests.agent("lander-1", role: "lander", model: "opus")
        lander["spend"] = ["input_tokens": 1_000_000, "output_tokens": 400_000]
        let plan = try PlanModelTests.plan(
            themes: [], lines: [], landings: [PlanModelTests.landing(9, "pushed", sha: "3f78754fabc", since: 6_480_000, lander: "lander-1")],
            agents: [lander])
        let card = try #require(PlanView.build(plan: plan)?.landings.first)
        #expect(card.facts(ci: nil, now: 7_200_000) == "3f78754f \u{00B7} CI not read yet \u{00B7} 12 min \u{00B7} Lander \u{00B7} opus \u{00B7} 1.4M")
        #expect(card.factsUnbroken(ci: nil, now: 7_200_000).contains("12\u{00A0}min"))
    }
}
