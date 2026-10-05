import Testing

@testable import AgentKit

/// The tree's status filter reads "Not Done", not "Open" (ov-330, R-13).
struct OneTreeFilterTests {
    @Test("The first choice is titled Not Done, and still stored as open, so a kept choice still reads")
    func titles() {
        #expect(OneTreeFilter.allCases.map(\.title) == ["Not Done", "In Review", "All"])
        #expect(OneTreeFilter.open.rawValue == "open")
        #expect(OneTreeFilter(rawValue: "open") == .open)
    }

    @Test("Not Done is what the tree shows of an unfinished card, review included")
    func notDoneIncludesReview() {
        #expect(OneTreeFilter.open.shows(.inReview))
        #expect(OneTreeFilter.open.shows(.inProgress))
        #expect(!OneTreeFilter.open.shows(.done))
        #expect(!OneTreeFilter.inReview.shows(.inProgress))
    }

    @Test("Each choice says what it shows, and the control's tooltip says all three")
    func help() {
        #expect(OneTreeFilter.open.help.contains("review"))
        #expect(OneTreeFilter.allCases.allSatisfy { !$0.help.isEmpty })
        #expect(OneTreeFilter.allHelp.hasPrefix("Not Done: "))
        #expect(OneTreeFilter.allHelp.contains("\nIn Review: "))
    }

    @Test("A page's \"open\" card count says Not Done, as the filter does")
    func pageCountWord() {
        #expect(PageWorld.statusName("open") == OneTreeFilter.open.title)
        #expect(PageWorld.statusName("in_review") == "In Review")
    }
}
