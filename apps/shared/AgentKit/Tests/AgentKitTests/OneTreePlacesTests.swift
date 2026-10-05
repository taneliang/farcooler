import Testing

@testable import AgentKit

/// The tree's pinned places (R-14, ov-332): Needs You, then Plan with its
/// pages. The orchestrator is the chat column, not a row.
struct OneTreePlacesTests {
    @Test("No Orchestrator row, even while the orchestrator itself asks; Needs You leads, then Plan")
    func noOrchestratorRow() throws {
        var asks = OneTreeAsks()
        asks.orchestrator = true
        let tree = OneTree.build(OneTreeInput(tasks: [], asks: asks, needsYouCount: 1))
        #expect(tree.places.map(\.title) == ["Needs You", "Plan"])
        #expect(tree.places.allSatisfy { $0.target != .orchestrator })
        #expect(tree.allNodes.allSatisfy { $0.id != "place:orchestrator" })
        #expect(tree.places[0].detail == "1")
    }

    @Test("A page under Plan has its tooltip text; it says what a page is")
    func pageHelp() {
        #expect(OneTreeWords.pageHelp == "A page the orchestrator wrote to explain where things stand. It updates as the work does.")
        let tree = OneTree.build(OneTreeInput(tasks: [], pages: [OneTreePage(slot: "train", title: "Train")]))
        let page = tree.places[1].children.first
        #expect(page?.kind == .page)
    }
}
