import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Amber is for attention alone (ov-269 design 6.2, ov-284): a page's list
/// with a question still waiting on the owner draws amber; the same list once
/// it's answered draws none. Rendered at 1x and 2x (ov-280), counting pixels
/// whose red leads blue by far more than any gray, glyph edge or the accent's
/// blue can: antialiasing mixes amber with the paper, which lowers that lead
/// but never makes a gray pixel lead, so the count holds at either scale.
@MainActor
struct PageLookTests {
    static func amber(status: TaskStatus, scale: Int, dark: Bool) throws -> Int {
        let world = PageWorld(tasks: [TaskRow(id: "t", key: "ov-222", title: "Tint", status: status, statusSince: Date())])
        let doc = PageDoc(title: "Risks", blocks: [
            .list([PageItem(text: "Sidebar tint is undecided", state: .blocked, detail: "Holds the last pass", ref: PageRef(.ask("ov-222")), tone: .neutral)])
        ])
        let host = NSHostingView(rootView: PageBlocksView(doc: doc, world: world, width: 480, onOpen: { _ in })
            .padding(Spacing.section).frame(width: 480, height: 120).background(Color(nsColor: .textBackgroundColor)))
        host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        host.frame = CGRect(x: 0, y: 0, width: 480, height: 120)
        host.layoutSubtreeIfNeeded()
        let rep = try #require(host.lookBitmap(scale: scale))
        #expect(rep.lookScale == scale)
        var count = 0
        for y in 0..<Int(rep.size.height) {
            for x in 0..<Int(rep.size.width) {
                let c = rep.color(atPoint: x, y)
                if c.redComponent - c.blueComponent > 0.3 && c.redComponent > c.greenComponent { count += 1 }
            }
        }
        return count
    }

    @Test("an open question draws amber; answered, the page draws none", arguments: lookScales)
    func amberOnlyWhileItWaits(scale: Int) throws {
        for dark in [false, true] {
            #expect(try Self.amber(status: .needsDecision, scale: scale, dark: dark) > 3, "dark \(dark)")
            #expect(try Self.amber(status: .inProgress, scale: scale, dark: dark) == 0, "dark \(dark)")
        }
    }
}
