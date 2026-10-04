import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Needs You items are unstroked cards, and the History page's area chips are
/// pills that state a filter (ov-225).
@MainActor
struct NeedsYouLookTests {
    private func bitmap<V: View>(_ view: V, size: CGSize, _ name: NSAppearance.Name = .aqua) throws -> NSBitmapImageRep {
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height, alignment: .topLeading))
        host.appearance = NSAppearance(named: name)
        host.frame = CGRect(origin: .zero, size: size)
        host.layoutSubtreeIfNeeded()
        let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        return rep
    }

    private func color(_ rep: NSBitmapImageRep, _ x: Int, _ y: Int) throws -> NSColor {
        try #require(rep.colorAt(x: x * 2, y: y * 2)).usingColorSpace(.sRGB)!
    }

    private let item = NeedsYouItem(
        id: "ask:x", kind: .ask, rank: 1, since: nil, workspaceID: "ws", workspaceName: "Billing", repositoryID: "r",
        task: nil, terminal: nil, question: "Allow touch x?", askID: nil, actions: [])

    @Test("A Needs You card has no stroke: its edge is the paper's color")
    func noStroke() throws {
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let row = NeedsYouItemRow(
                item: item, canAct: true, onOpen: {}, onAnswerAsk: { _ in nil }, onDecide: { _ in true })
            let rep = try bitmap(row, size: CGSize(width: 400, height: 90), name)
            // On the left edge at mid height, against just inside it.
            #expect(try color(rep, 0, 40) == color(rep, 4, 40), "an edge is drawn in \(name.rawValue)")
        }
    }

    private struct Chips: View {
        @State private var chosen: String? = "Mac"
        var body: some View { AreaChips(areas: ["Mac", "iOS"], chosen: $chosen).padding(8) }
    }

    @Test("The chosen chip is a pill in the selection's wash, not a solid accent fill")
    func theChosenChipIsAWash() throws {
        let rep = try bitmap(Chips(), size: CGSize(width: 160, height: 40))
        var solid = 0
        for x in 0..<rep.pixelsWide {
            for y in 0..<rep.pixelsHeight {
                guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                if c.alphaComponent > 0.9, c.blueComponent > 0.8, c.redComponent < 0.3, c.greenComponent < 0.6 { solid += 1 }
            }
        }
        #expect(solid == 0, "a solid accent fill is drawn: \(solid) px")
    }
}

private extension NSBitmapImageRep {
    var pixelsHeight: Int { pixelsHigh }
}
