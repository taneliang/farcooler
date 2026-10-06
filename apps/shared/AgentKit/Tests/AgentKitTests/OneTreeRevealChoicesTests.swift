import Foundation
import Testing

@testable import AgentKit

/// Revealing a selection opens its ancestors for drawing, not as the
/// person's own choices (ov-345, jumpbar-icons review R2-2).
struct OneTreeRevealChoicesTests {
    typealias T = OneTreeTests

    /// Mac's card t3, in the quiet board: its theme is closed by default.
    private func reveal() throws -> (tree: OneTree, ids: Set<String>, target: OneTreeTarget) {
        let tree = OneTree.build(try T.quietBoard())
        let card = try #require(T.node(tree, "theme:theme-Mac/task:t3"))
        let target = try #require(card.target)
        let ids = Set(tree.ancestors(of: target))
        #expect(ids.contains("theme:theme-Mac"), "the card sits under its theme: \(ids)")
        return (tree, ids, target)
    }

    @Test("A revealed ancestor is drawn open, and nothing is recorded as the person's choice")
    func revealedIsDrawnNotKept() throws {
        let (tree, ids, target) = try reveal()
        let kept = OneTreeExpansion()
        let drawn = kept.revealing(ids)
        #expect(OneTree.rows(tree.tree, expansion: drawn).contains { $0.node.target == target }, "in sight")
        #expect(!OneTree.rows(tree.tree, expansion: kept).contains { $0.node.target == target }, "closed otherwise")
        // The person then folds something unrelated: only that is recorded.
        var next = drawn
        next.choices["group:loose"] = true
        let after = kept.recording(next, over: drawn)
        #expect(after.choices.choices == ["group:loose": true])
        #expect(after.touched == ["group:loose"])
    }

    @Test("Once the work on a revealed theme lands, it follows the live default again")
    func followsTheDefaultAfterward() throws {
        var input = try T.quietBoard()
        let (_, ids, _) = try reveal()
        // The window revealed Mac while a lane was building there, then recorded some
        // unrelated action over it.
        input.plan.lanes[1].state = .building
        let busy = OneTree.build(input)
        let drawn = OneTreeExpansion().revealing(ids)
        var next = drawn
        next.choices["group:loose"] = false
        let kept = OneTreeExpansion().recording(next, over: drawn).choices
        #expect(kept.choices["theme:theme-Mac"] == nil)
        // The work lands and the selection goes elsewhere: Mac is closed again.
        let quiet = OneTree.build(try T.quietBoard())
        let mac = try #require(T.node(quiet, "theme:theme-Mac"))
        #expect(!OneTree.rows(quiet.tree, expansion: kept).first { $0.id == mac.id }!.expanded)
        #expect(OneTree.rows(busy.tree, expansion: kept).first { $0.id == "theme:theme-Mac" }!.expanded, "and open while it was live")
    }

    @Test("The person closing a revealed ancestor is their choice, and is recorded")
    func closingARevealedAncestorIsRecorded() throws {
        let (tree, ids, _) = try reveal()
        let drawn = OneTreeExpansion().revealing(ids)
        let mac = try #require(T.node(tree, "theme:theme-Mac"))
        var next = drawn
        next.toggle(mac)
        let after = OneTreeExpansion().recording(next, over: drawn)
        #expect(after.choices.choices["theme:theme-Mac"] == false)
        #expect(after.touched.contains("theme:theme-Mac"))
    }
}
