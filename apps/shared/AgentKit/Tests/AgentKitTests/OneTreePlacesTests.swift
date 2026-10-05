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

/// Collapse All, Expand All and ⌥-click (ov-334).
struct OneTreeFoldTests {
    private static func tree() throws -> OneTree {
        OneTree.build(try OneTreeTests.board())
    }

    @Test("Collapse All closes every node with children, and the defaults don't come back")
    func collapseAll() throws {
        let tree = try Self.tree()
        var expansion = OneTreeExpansion()
        #expect(expansion.anyExpanded(in: tree.roots))
        expansion.setAll(false, in: tree.roots)
        #expect(!expansion.anyExpanded(in: tree.roots))
        #expect(OneTree.rows(tree.roots, expansion: expansion).allSatisfy { !$0.expanded })
        // Activity can't open one again: the choice is kept as text.
        #expect(!OneTreeExpansion(encoded: expansion.encoded).anyExpanded(in: tree.roots))
    }

    @Test("Expand All opens the themes, cards and lanes to the terminals, and leaves done folds and Hidden shut")
    func expandAll() throws {
        let tree = try Self.tree()
        var expansion = OneTreeExpansion()
        expansion.setAll(false, in: tree.roots)
        expansion.setAll(true, in: tree.roots)
        let rows = OneTree.rows(tree.roots, expansion: expansion)
        for row in rows where row.node.hasChildren {
            switch row.node.kind {
            case .theme, .task, .lane: #expect(row.expanded, "\(row.node.kind) \(row.node.id)")
            case .doneFold: #expect(!row.expanded, "a done fold stays shut: \(row.node.id)")
            default: break
            }
        }
        #expect(rows.contains { $0.node.kind == .terminal })
        #expect(!rows.contains { $0.node.kind == .task && $0.node.quiet && $0.node.id.contains("/done") })
        let hidden = tree.allNodes.filter { $0.title == OneTreeWords.hidden }
        for node in hidden { #expect(!expansion.isExpanded(node)) }
        let loose = try #require(tree.below.first { $0.title == OneTreeWords.looseWorktrees })
        #expect(!expansion.isExpanded(loose))
    }

    @Test("⌥-click on a disclosure takes its siblings the same way")
    func optionClick() throws {
        let tree = try Self.tree()
        let plan = try #require(tree.tree.first { $0.id == "theme:theme-Plan" })
        var expansion = OneTreeExpansion()
        expansion.setAll(false, in: tree.roots)
        expansion.toggle(plan, withSiblings: tree.siblings(of: plan.id))
        // Every theme with children is open now; the groups below are not its siblings.
        for sibling in tree.tree where sibling.hasChildren { #expect(expansion.isExpanded(sibling), "\(sibling.id)") }
        let checkout = try #require(tree.below.first)
        #expect(!expansion.isExpanded(checkout))
        // Again: all of them close.
        expansion.toggle(plan, withSiblings: tree.siblings(of: plan.id))
        for sibling in tree.tree where sibling.hasChildren { #expect(!expansion.isExpanded(sibling), "\(sibling.id)") }
    }

    @Test("Siblings are the parent's children, or the group at the top")
    func siblings() throws {
        let tree = try Self.tree()
        #expect(tree.siblings(of: "theme:theme-Plan").map(\.id) == tree.tree.map(\.id))
        #expect(tree.siblings(of: "place:plan").map(\.id) == tree.places.map(\.id))
        let task = try #require(tree.allNodes.first { $0.id.hasPrefix("theme:theme-Plan/task:") && $0.kind == .task })
        let parent = try #require(tree.tree.first { $0.id == "theme:theme-Plan" })
        #expect(tree.siblings(of: task.id).map(\.id) == parent.children.map(\.id))
        #expect(tree.siblings(of: "nowhere").isEmpty)
    }
}
