import Foundation
import Testing

@testable import AgentKit

/// The phones' real bytes in `swift test` (ov-285 review): a seeded daemon's
/// own `page list --json`, `plan --json` and cards, the file the iPhone's
/// harness serves, through the reader and the live references.
struct PageSeededTests {
    @Test("a seeded board's pages decode, and their references draw live")
    func seededPagesAreLive() throws {
        let data = try Data(contentsOf: PageModelTests.root.appendingPathComponent("test/fixtures/pages-seeded.json"))
        let object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let pages = try BoardPageList.decode(JSONSerialization.data(withJSONObject: try #require(object["pages"]))).pages
        #expect(pages.map(\.slot) == ["train", "spend", "risks"])
        #expect(pages.allSatisfy { $0.doc != nil })
        let plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: try #require(object["plan"])))
        let theme = try #require(plan.themes.first { $0.name == "Visual language" })
        #expect(PageShelf.anchored(pages, to: theme.id, plan: plan).map(\.slot) == ["risks"])
        let world = PageWorld(plan: plan, pages: pages)
        #expect(world.resolve(PageRef(.lane("ov-181-review"))).status == "Fixing · round 1 · train integ-10")
        #expect(world.resolve(PageRef(.page("spend"))).name == "Spend")
    }
}
