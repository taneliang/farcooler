import Foundation
import Testing

@testable import AgentKit

/// Engine landing groups are what the Plan's Landing section shows (ov-1412):
/// a runner's stored landings feed nothing there, alone or beside the groups.
struct PlanEngineLandingTests {
    /// The shared four-group queue, with `historical` stored landings added.
    private static func plan(historical: [[String: Any]] = [], engine: Bool = true) throws -> PlanModel {
        let url = PlanPhoneFixtures.root.appendingPathComponent("test/fixtures/plan-landing-queue.json")
        var object = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
        object["landings"] = historical
        if !engine { object["workflow"] = nil }
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
    }

    private static func groups(_ plan: PlanModel) throws -> [WireLandingGroupView] {
        let view = try #require(WirePlanView.build(plan: plan))
        return try #require(view.landingGroups)
    }

    @Test("a group reads its step, its time in it, then each ticket aboard")
    func readingOrder() throws {
        let group = try #require(try Self.groups(Self.plan()).first)
        #expect(group.title == "Landing group 1")
        #expect(group.lines == [
            "Run gate \u{00B7} Running",
            "4 min in this step",
            "ov-1 \u{00B7} Keep the complete long ticket title visible when the landing queue wraps on a narrow phone screen",
            "ov-2",
        ])
    }

    @Test("a split group says so, then names the failed job and its first line")
    func splitFailure() throws {
        let group = try #require(try Self.groups(Self.plan()).first { $0.title == "Landing group 3" })
        #expect(Array(group.lines.suffix(3)) == [
            "Split after a gate failed",
            "Failed job: cargo test --workspace",
            "error: expected a passing gate",
        ])
    }

    @Test("a group with no start time says the time is unavailable")
    func noStartTime() throws {
        let group = try #require(try Self.groups(Self.plan()).first { $0.title == "Landing group 4" })
        #expect(group.lines.dropFirst().first == "Time unavailable")
        #expect(!group.lines.contains("Split after a gate failed"))
    }

    @Test("stored landings beside engine groups change neither the groups nor the Landing heading")
    func historicalBesideEngine() throws {
        let alone = try #require(WirePlanView.build(plan: try Self.plan()))
        let plan = try Self.plan(historical: [PlanModelTests.landing(86, "pushed", sha: "abc", since: 5)])
        #expect(plan.landings.count == 1)
        let mixed = try #require(WirePlanView.build(plan: plan))
        #expect(mixed.landings.isEmpty)
        #expect(mixed.landingGroups == alone.landingGroups && mixed.landingGroups?.count == 4)
        #expect(mixed.landingGroupHeading == "Landing" && mixed.landingGroupHeading == alone.landingGroupHeading)
        #expect(mixed.section(.landing) == alone.section(.landing))
    }

    @Test("a runner with only stored landings gets no engine groups and no group heading")
    func historicalOnly() throws {
        let plan = try Self.plan(historical: [PlanModelTests.landing(86, "pushed")], engine: false)
        let view = try #require(WirePlanView.build(plan: plan))
        #expect(view.landings.isEmpty)
        #expect(view.landingGroups?.isEmpty ?? true)
        #expect(view.landingGroupHeading == nil)
    }
}
