import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Cost on the Mac's Plan view (ov-307): a theme past its token budget says so
/// in the navigator, in the plan's own words, a theme within it or with none
/// says nothing, and the runner's week and comparison sit in a Cost section,
/// drawn only when the runner sent them. The words are AgentKit's
/// (`PlanCostTests`); this holds the real views to them.
@MainActor
@Suite(.serialized)
struct PlanCostViewTests {
    /// The fixture with `edit` applied to its first theme and its plan.
    static func plan(_ edit: (inout [String: Any], inout [String: Any]) -> Void) throws -> Data {
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        var themes = try #require(object["themes"] as? [[String: Any]])
        edit(&object, &themes[0])
        object["themes"] = themes
        return try JSONSerialization.data(withJSONObject: object)
    }

    /// The fixture with `edit` applied to the whole plan.
    static func edit(_ edit: (inout [String: Any]) throws -> Void) throws -> Data {
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        try edit(&object)
        return try JSONSerialization.data(withJSONObject: object)
    }

    /// The views drawn once the overview has settled over `plan`.
    static func drawn(_ plan: Data) async throws -> [String: CGRect] {
        let calls = PlanViewTests.Calls()
        calls.plan = plan
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        let seen = NavigatorFilterTests.Seen()
        let host = NSHostingView(
            rootView: PlanChangesTests.Hosted(
                store: store, selection: PlanChangesTests.Selection(), seen: seen, heard: PlanChangesTests.Heard()))
        host.frame = CGRect(x: 0, y: 0, width: 320, height: 900)
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return seen.views
    }

    @Test("A theme past its budget says so on its track line; within it, or with no budget, it says nothing")
    func aThemePastItsBudgetSaysSo() async throws {
        let over = try await Self.drawn(PlanViewTests.fixture())
        #expect(over["plan-theme-entry-Visual language-over-budget"] != nil, "320K of a 250K budget: \(over.keys.sorted())")

        let within = try await Self.drawn(Self.plan { _, theme in theme["budget_tokens"] = 400_000 })
        #expect(within["plan-theme-entry-Visual language-head"] != nil, "the theme is still drawn")
        #expect(within["plan-theme-entry-Visual language-over-budget"] == nil, "within its budget draws no warning")

        let none = try await Self.drawn(Self.plan { _, theme in theme["budget_tokens"] = NSNull() })
        #expect(none["plan-theme-entry-Visual language-over-budget"] == nil, "no budget, nothing to flag")
    }

    @Test("A lane that waits on you and is over its budget says both")
    func aLaneSaysBothWarnings() async throws {
        // mac-ux's card ov-1 needs a decision, so it waits on the owner; give it a budget it is past.
        let both = try await Self.drawn(
            try Self.edit { object in
                var lanes = try #require(object["lanes"] as? [[String: Any]])
                for index in lanes.indices where lanes[index]["name"] as? String == "mac-ux" { lanes[index]["budget_tokens"] = 100_000 }
                object["lanes"] = lanes
            })
        #expect(both["plan-lane-mac-ux-warning"] != nil, "Needs you is still said: \(both.keys.sorted())")
        #expect(both["plan-lane-mac-ux-over-budget"] != nil, "and so is the budget")
        let within = try await Self.drawn(PlanViewTests.fixture())
        #expect(within["plan-lane-mac-ux-warning"] != nil && within["plan-lane-mac-ux-over-budget"] == nil, "within 500K: only Needs you")
    }

    @Test("The Cost section is there when the runner sent cost, and not when it didn't")
    func theCostSectionFollowsTheRunner() async throws {
        let sent = try await Self.drawn(PlanViewTests.fixture())
        #expect(sent["plan-cost-section"] != nil, "the fixture's runner sends cost: \(sent.keys.sorted())")
        let withheld = try await Self.drawn(Self.plan { plan, theme in
            plan["cost"] = NSNull()
            theme["budget_tokens"] = NSNull()
            theme["trend_tokens"] = NSNull()
        })
        #expect(withheld["plan-cost-section"] == nil, "a runner without board_cost sends none")
        #expect(withheld["plan-theme-entry-Visual language-over-budget"] == nil)
    }

    @Test("The week's total and its split by harness and model take room in the Cost block; without the split, only the week line")
    func theWeekSplitIsDrawn() throws {
        let cost = try #require(try PlanModel.decode(PlanViewTests.fixture()).cost)
        #expect(cost.week.count == 2 && cost.weekCostMicros == 45_000_000, "the fixture's runner sends the split")
        func height(_ cost: PlanCostRead) -> CGFloat {
            let host = NSHostingView(rootView: PlanCostBlock(cost: cost).frame(width: 300))
            host.frame = CGRect(x: 0, y: 0, width: 300, height: 2000)
            host.layoutSubtreeIfNeeded()
            return host.fittingSize.height
        }
        var older = cost
        older.week = []
        older.weekCostMicros = nil
        // The dollars line and two two-line rows: well over three lines of 13 pt text.
        #expect(height(cost) - height(older) > 80, "\(height(cost)) against \(height(older))")
    }
}
