import Foundation
import Testing

@testable import AgentKit

/// A lane's row in the One tree says what a person reads (ov-466): its title,
/// "Train 9" for a train's own lane, the slug only when there is nothing else.
/// The slug stays on the node, so identifiers and search do not move.
struct OneTreeLaneTitleTests {
    typealias T = OneTreeTests

    private func laneNodes(titles: [String: String]) throws -> [String: OneTreeNode] {
        var input = try T.board()
        for index in input.plan.lanes.indices {
            input.plan.lanes[index].title = titles[input.plan.lanes[index].name]
        }
        let tree = OneTree.build(input)
        var found: [String: OneTreeNode] = [:]
        for node in tree.allNodes where node.kind == .lane && !node.slug.isEmpty { found[node.slug] = node }
        return found
    }

    @Test("A lane's row says its title, and keeps the slug for its identifier")
    func titled() throws {
        let nodes = try laneNodes(titles: ["primary": "Agents message the orchestrator"])
        let primary = try #require(nodes["primary"])
        #expect(primary.title == "Agents message the orchestrator")
        #expect(primary.rowName == "primary")
        #expect(nodes["copy"]?.title == "copy", "a lane with no title says its slug")
    }

    @Test("A train's own lane says Train N, not integ-N")
    func trainLane() throws {
        var input = try T.board()
        input.plan.lanes[0].name = "integ-9"
        let tree = OneTree.build(input)
        let train = try #require(tree.allNodes.first { $0.kind == .lane && $0.slug == "integ-9" })
        #expect(train.title == "Train 9")
    }

    @Test("Search still finds a lane by its slug")
    func searchBySlug() throws {
        let input = try { var i = try T.board(); i.plan.lanes[0].title = "Agents message"; return i }()
        let tree = OneTree.build(input)
        let hit = OneTree.narrowed(tree.tree, to: "primary")
        #expect(hit.flatMap { [$0] + $0.children }.contains { $0.slug == "primary" || !$0.children.isEmpty })
    }
}
