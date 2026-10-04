import Testing

@testable import AgentKit

/// One banner per pane, and a closed pane's goes with it (ov-163, ov-153).
struct PaneBannerTests {
    @Test func everyStateOfOnePaneIsOneIdentifier() {
        // The center replaces by identifier, so blocked, done and failed for
        // one pane must be the same string, and two panes must differ.
        #expect(PaneBanner.identifier(forPane: "t-1") == PaneBanner.identifier(forPane: "t-1"))
        #expect(PaneBanner.identifier(forPane: "t-1") != PaneBanner.identifier(forPane: "t-2"))
        #expect(!PaneBanner.identifier(forPane: "t-1").contains("-blocked"))
    }

    @Test func closingRemovesWhatWasPostedNowAndBefore() {
        let removed = PaneBanner.removing(pane: "t-1")
        #expect(removed.contains(PaneBanner.identifier(forPane: "t-1")), "the banner posted now")
        #expect(removed.contains("t-1-blocked") && removed.contains("t-1-done") && removed.contains("t-1-failedRun"))
        #expect(!removed.contains { $0.hasPrefix("t-2") })
    }

    @Test func aPaneThatVanishedFromTheFleetIsClosed() {
        #expect(PaneBanner.closed(before: ["a", "b", "c"], after: ["b"]) == ["a", "c"])
        #expect(PaneBanner.closed(before: [], after: ["a"]).isEmpty, "a new pane is not a closed one")
        #expect(PaneBanner.closed(before: ["a"], after: ["a"]).isEmpty)
    }
}
