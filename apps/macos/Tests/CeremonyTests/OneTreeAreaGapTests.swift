import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Every area under a rule keeps the rhythm's gap between the rule and its
/// first row (ov-406). The places and the shells have no header of their own,
/// and the room over a header is padding on it, which SwiftUI doesn't draw on
/// an empty view: the shells' first row sat right under the line.
@MainActor
@Suite(.serialized)
struct OneTreeAreaGapTests {
    @Test("The shells' first row is a rhythm under the rule over them, at a short and a tall window")
    func shellsKeepTheirGap() async throws {
        for height: CGFloat in [420, 900] {
            let seen = await OneTreeSplitWindowTests.draw(height: height, tasks: 40)
            let tree = try #require(seen.frames["navigator-pane-tree"], "\(height): \(seen.frames.keys.sorted())")
            let shells = try #require(seen.frames["navigator-pane-shells"])
            // A pane's scroll view ends where the rule starts (NavigatorSplitTests),
            // so the rule's line is the slot after it. The pane's content starts at
            // its scroll view's top, so the gap is the header's room over it. At 1x
            // a point is a pixel, so half a point is the rounding there is.
            let gap = shells.minY - (tree.maxY + NavigatorSplit.ruleSlot)
            #expect(abs(gap - NavigatorSplit.headerInset) <= 0.5, "\(height): \(gap) under the rule, wanted \(NavigatorSplit.headerInset)")
        }
    }
}
