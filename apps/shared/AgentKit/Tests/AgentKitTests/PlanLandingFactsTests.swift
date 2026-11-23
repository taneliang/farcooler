import Foundation
import Testing

@testable import AgentKit

/// Historical landing facts remain in the model but are not presented.
struct PlanLandingFactsTests {
    @Test("historical landing facts are not presented")
    func historicalLandingIsAbsent() throws {
        let plan = try PlanModelTests.plan(
            themes: [], lines: [], landings: [PlanModelTests.landing(9, "pushed", sha: "3f78754fabc", since: 6_480_000)])
        #expect(!plan.landings.isEmpty)
        #expect(try #require(WirePlanView.build(plan: plan)).landings.isEmpty)
    }
}
