import Foundation
import Testing

@testable import AgentKit

/// Where a board's pages are drawn (design 6.1, ov-285): in the Pages
/// section, or inside the live theme they're anchored to, and never lost when
/// that theme is gone.
struct PageShelfTests {
    static func page(_ slot: String, anchor: String? = nil) -> BoardPage {
        BoardPage(id: slot, slot: slot, title: slot, anchorKind: anchor == nil ? "" : "theme", anchor: anchor ?? "")
    }

    @Test("a page of its own is listed; an anchored one is drawn in its theme, and only there")
    func listedAndAnchored() throws {
        let plan = try PlanModelTests.plan(
            themes: [
                PlanModelTests.theme("Visual", cards: [], ordinal: 0),
                PlanModelTests.theme("Gone", cards: [], ordinal: 1, state: "dropped"),
            ], lanes: [])
        let pages = [
            Self.page("train"), Self.page("risks", anchor: "theme-Visual"), Self.page("old", anchor: "theme-Gone"),
            Self.page("lost", anchor: "theme-Nowhere"),
        ]
        #expect(PageShelf.listed(pages, plan: plan).map(\.slot) == ["train", "old", "lost"])
        #expect(PageShelf.anchored(pages, to: "theme-Visual", plan: plan).map(\.slot) == ["risks"])
        #expect(PageShelf.anchored(pages, to: "theme-Gone", plan: plan).isEmpty)
    }

    @Test("without a plan, every page is listed and none is anchored")
    func withoutAPlan() {
        let pages = [Self.page("train"), Self.page("risks", anchor: "theme-Visual")]
        #expect(PageShelf.listed(pages, plan: nil).map(\.slot) == ["train", "risks"])
        #expect(PageShelf.anchored(pages, to: "theme-Visual", plan: nil).isEmpty)
    }
}
