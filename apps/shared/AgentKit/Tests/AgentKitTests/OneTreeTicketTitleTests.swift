import Foundation
import Testing

@testable import AgentKit

/// A ticket's row in the One tree says its key (ov-466, ov-872), the same on
/// both phones. The key is also what identifiers and search go by; the branch
/// is not said, since the card above the row has the words.
struct OneTreeTicketTitleTests {
    typealias T = OneTreeTests

    private func ticketNodes(branches: [String: String]) throws -> [String: OneTreeNode] {
        var input = try T.board()
        for index in input.plan.lines.indices {
            input.plan.lines[index].branch = branches[input.plan.lines[index].key] ?? ""
        }
        let tree = OneTree.build(input)
        var found: [String: OneTreeNode] = [:]
        for node in tree.allNodes where node.kind == .ticket && !node.slug.isEmpty { found[node.slug] = node }
        return found
    }

    @Test("A ticket's row says its key, whatever its branch is, and keeps the key for its identifier")
    func keyed() throws {
        let nodes = try ticketNodes(branches: ["ov-1": "agents-message-the-orchestrator"])
        let first = try #require(nodes["ov-1"])
        #expect(first.title == "ov-1" && first.rowName == "ov-1")
        #expect(nodes["ov-2"]?.title == "ov-2" && nodes["ov-2"]?.rowName == "ov-2", "with a branch or without")
    }

    @Test("A ticket's row is the card's own: one row per ticket, all keyed by the card")
    func oneRowPerTicket() throws {
        let nodes = try ticketNodes(branches: [:])
        #expect(nodes.keys.sorted() == ["ov-1", "ov-2", "ov-3"])
        #expect(nodes.values.allSatisfy { $0.title == $0.slug && $0.target != nil })
    }

    @Test("Search finds a ticket by its key")
    func searchByKey() throws {
        let tree = OneTree.build(try T.board())
        let hit = OneTree.narrowed(tree.tree, to: "ov-2")
        let found = hit.flatMap { $0.children }.flatMap { $0.children }
        #expect(found.contains { $0.kind == .ticket && $0.slug == "ov-2" })
    }
}
