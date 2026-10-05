import Foundation
import SwiftUI
import Testing

@testable import AgentKit

/// The page's layout rules (ov-269 design 3.6, 3.7; ov-284): a wide table
/// stacks on a narrow surface, a row speaks with its titles, a text block
/// draws only the Markdown subset, and a timeline is newest first.
@MainActor
struct PageLayoutTests {
    @Test("more than three columns stack under 480 points; three or fewer never do")
    func stackingRule() {
        #expect(PageLayout.stacks(columns: 4, width: 400))
        #expect(!PageLayout.stacks(columns: 4, width: 480))
        #expect(!PageLayout.stacks(columns: 3, width: 300))
        #expect(!PageLayout.stacks(columns: 8, width: nil))
        #expect(PageLayout.stepsDown(width: 300) && !PageLayout.stepsDown(width: 700))
    }

    @Test("a row reads with its column titles, live values included, empty cells left out")
    func spokenRow() throws {
        let world = try PageLiveTests.world()
        let columns = ["Lane", "Cards", "Gate", "State"].map { PageColumn(title: $0, align: .start, grow: false) }
        let row = [
            PageCell(ref: PageRef(.lane("ov-274-phones"))), PageCell(ref: PageRef(.task("ov-274"))), PageCell(text: ""),
            PageCell(ref: PageRef(.lane("ov-274-phones")), show: .state),
        ]
        #expect(
            PageLayout.spokenRow(columns: columns, cells: row, world: world)
                == "Lane, ov-274-phones. Cards, ov-274. State, In Review · train integ-10.")
    }

    @Test("a text block draws paragraphs and lists two deep; headings, fences and quotes are plain words")
    func markdownSubset() {
        let pieces = PageMarkdown.pieces("# Title\n\nA **bold** line.\n\n- one\n  - two\n    - three\n\n```\ncode\n```\n\n> quoted")
        #expect(
            pieces == [
                .plain("Title"), .prose("A **bold** line."), .item(marker: "•", text: "one", depth: 0),
                .item(marker: "•", text: "two", depth: 1), .item(marker: "•", text: "three", depth: 1), .plain("code"),
                .plain("quoted"),
            ])
    }

    @Test("only https and task links stay links in text; the words stay either way")
    func inlineLinks() {
        let text = PageMarkdown.inline("[a](https://github.com/x) [b](http://github.com/x) [c](mailto:a@b.c) [d](file:///etc)")
        let links = text.runs.compactMap { run in run.link.map { (String(text[run.range].characters), $0.absoluteString) } }
        #expect(links.map(\.0) == ["a"])
        #expect(String(text.characters) == "a b c d")
    }

    @Test("a timeline is newest first unless given; ties keep their order")
    func timelineOrder() {
        let entries = [PageEntry(at: 1, text: "a"), PageEntry(at: 3, text: "b"), PageEntry(at: 3, text: "c"), PageEntry(at: 2, text: "d")]
        #expect(PageLayout.ordered(entries, given: false).map(\.text) == ["b", "c", "d", "a"])
        #expect(PageLayout.ordered(entries, given: true).map(\.text) == ["a", "b", "c", "d"])
    }

    @Test("a timeline's time is in the viewer's zone, with the date before today")
    func timelineTime() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try #require(TimeZone(identifier: "America/Los_Angeles"))
        // 2026-10-04 15:02 in Los Angeles is 22:02 UTC.
        let at: Int64 = 1_791_151_320_000
        let sameDay = PageLayout.time(at, now: at + 3_600_000, calendar: calendar)
        let dayAfter = PageLayout.time(at, now: at + 86_400_000, calendar: calendar)
        #expect(sameDay.contains("3:02") || sameDay.contains("15:02"))
        #expect(dayAfter.contains("Oct") && dayAfter.contains("4"))
    }

    #if os(macOS)
    /// A five-column table measured at a phone's width and at a window's: the
    /// stacked one puts each value on its own line, so it's several times
    /// the grid's height. Heights of the same text in the same environment,
    /// so CI's metrics move both alike.
    @Test("the five-column table draws stacked at 360 points and as a grid at 720")
    func theTableStacksWhenNarrow() throws {
        let doc = try PageModelTests.doc("refs")
        let table = PageDoc(title: "T", blocks: doc.blocks.filter { if case .table = $0 { true } else { false } })
        let world = try PageLiveTests.world()
        func height(_ width: CGFloat) -> CGFloat {
            let renderer = ImageRenderer(
                content: PageBlocksView(doc: table, world: world, width: width, onOpen: { _ in }).frame(width: width))
            return renderer.nsImage?.size.height ?? 0
        }
        let narrow = height(360), wide = height(720)
        #expect(wide > 0)
        #expect(narrow > 2 * wide, "narrow \(narrow), wide \(wide)")
    }

    @Test("every fixture draws, references to things gone included, and an empty world draws too")
    func everyFixtureDraws() throws {
        for name in ["train", "spend", "risks", "blocks", "refs"] {
            let doc = try PageModelTests.doc(name)
            for world in [try PageLiveTests.world(), PageWorld()] {
                let page = BoardPage(id: name, slot: name, title: doc.title, doc: doc)
                let renderer = ImageRenderer(content: PageView(page: page, world: world, onOpen: { _ in }).frame(width: 640))
                let size = try #require(renderer.nsImage?.size)
                #expect(size.height > 100, "\(name)")
            }
        }
    }
    #endif
}
