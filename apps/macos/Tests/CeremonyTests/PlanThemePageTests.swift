import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The theme's page and the footer under the Plan's themes. (The canvas's
/// theme entries, their folds and the 680-pt reading width went with the
/// themes grid, ov-503; `PlanSectionViewTests` holds the tiles.)
@MainActor
@Suite(.serialized)
struct PlanThemePageTests {
    /// What each probed view drew, in a column `width` wide.
    static func frames(_ view: some View, width: CGFloat, height: CGFloat = 1200) async -> [String: CGRect] {
        let seen = NavigatorFilterTests.Seen()
        let host = NSHostingView(
            rootView: view
                .frame(width: width, alignment: .topLeading)
                .fixedSize(horizontal: false, vertical: true)
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = seen.views = Dictionary(
                            probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                        Color.clear
                    }
                })
        host.frame = CGRect(x: 0, y: 0, width: width, height: height)
        for _ in 0..<5 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return seen.views
    }

    static func model(_ edit: (inout [String: Any], inout [String: Any]) -> Void = { _, _ in }) throws -> PlanModel {
        try PlanModel.decode(try PlanCostViewTests.plan(edit))
    }

    @Test("The theme's page reads in the entry's order, Where It Stands, Needs You, Next, and lists what was decided")
    func pageMatchesTheEntry() async throws {
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults())
        await store.plan.reload()
        let context = PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })
        let theme = try #require(store.plan.plan.themes.first)
        let drawn = await Self.frames(
            PlanPageView(plan: store.plan, page: .theme(theme.id), context: context), width: 800, height: 2400)
        let ask = try #require(drawn["plan-ask"], "\(drawn.keys.sorted())")
        let next = try #require(drawn["plan-theme-next"])
        let decided = try #require(drawn["plan-theme-decided"], "the theme's ruling R-2 is on its page")
        #expect(ask.minY < next.minY, "the ask comes before Next: \(ask) \(next)")
        #expect(next.minY < decided.minY, "Decided comes after")
    }

    @Test("The footer counts what's outside every theme, and lists the cards to tidy")
    func outsideFooter() async throws {
        let none = try await PlanCostViewTests.drawn(PlanViewTests.fixture())
        #expect(none["plan-outside"] == nil && none["plan-outside-tidy"] == nil, "every card is in a theme")
        let outside = try await PlanCostViewTests.drawn(
            PlanCostViewTests.edit { object in
                var themes = try #require(object["themes"] as? [[String: Any]])
                themes[0]["cards"] = []
                object["themes"] = themes
                object["no_lane"] = [["task": PlanViewTests.tasks[0].0, "key": "ov-1", "status": "in_review"]]
            })
        #expect(outside["plan-outside"] != nil, "open cards in no theme: \(outside.keys.sorted())")
        #expect(outside["plan-outside-tidy"] != nil, "and a cards-to-tidy button")
        let cards = [PlanFlaggedCard(task: "a", key: "ov-1", status: "in_review"), PlanFlaggedCard(task: "b", key: "ov-2", status: "in_progress")]
        let list = await Self.frames(PlanTidyList(cards: cards), width: 300)
        #expect(list["plan-tidy-ov-1"] != nil && list["plan-tidy-ov-2"] != nil, "each card is listed: \(list.keys.sorted())")
        let tidy = try #require(list["plan-tidy-ov-1"]).minY < (try #require(list["plan-tidy-ov-2"])).minY
        #expect(tidy, "in the CLI's order")
    }
}
