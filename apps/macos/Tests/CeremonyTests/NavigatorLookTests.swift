import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The navigator's rows share one look (ov-222): one selection fill, no hairline
/// around the filter, and "needs you" in amber, not the accent.
@MainActor
struct NavigatorLookTests {
    private func bitmap<V: View>(_ view: V, size: CGSize, _ name: NSAppearance.Name = .aqua, scale: Int) throws -> NSBitmapImageRep {
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
        host.appearance = NSAppearance(named: name)
        host.frame = CGRect(origin: .zero, size: size)
        host.layoutSubtreeIfNeeded()
        return try #require(host.lookBitmap(scale: scale))
    }

    private func color(_ rep: NSBitmapImageRep, _ x: Int, _ y: Int) throws -> NSColor {
        try #require(rep.colorAt(x: x * rep.pixelsWide / Int(rep.size.width), y: y * rep.pixelsHigh / Int(rep.size.height)))
            .usingColorSpace(.sRGB)!
    }

    private struct Row: View {
        var selected = false
        var keyed = false
        var highlighted = false
        var body: some View {
            CompactTaskRow(key: "ov-1", title: "A task", selected: selected, keyed: keyed, highlighted: highlighted) {
                Text("Backlog")
            }
            .padding(.horizontal, NavigatorGrid.edge)
            .environment(\.taskKeyWidth, 52)
        }
    }

    @Test("A row that just arrived is washed in the one selection fill", arguments: lookScales)
    func highlightIsTheSelection(scale: Int) throws {
        let size = CGSize(width: 320, height: 60)
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let selected = try bitmap(Row(selected: true, keyed: true), size: size, name, scale: scale)
            let arrived = try bitmap(Row(highlighted: true), size: size, name, scale: scale)
            // Right of the text, inside the row's box.
            #expect(try color(selected, 250, 30) == color(arrived, 250, 30), "\(name.rawValue)")
        }
    }

    private struct Filter: View {
        @FocusState private var focused: Bool
        @State private var text = ""
        var body: some View {
            NavigatorFilterField(text: $text, focused: $focused, onLeave: {})
                .padding(.horizontal, NavigatorGrid.edge)
        }
    }

    @Test("The filter's edge is its fill: nothing is stroked around it while it isn't focused", arguments: lookScales)
    func filterHasNoHairline(scale: Int) throws {
        let size = CGSize(width: 320, height: ColumnGrid.rowHeight)
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let rep = try bitmap(Filter(), size: size, name, scale: scale)
            // The top row of the box, and its middle, right of the placeholder.
            #expect(try color(rep, 250, 0) == color(rep, 250, Int(size.height / 2)), "\(name.rawValue)")
        }
    }

    @Test("An orchestrator that needs you says so in amber, not the accent, in its word", arguments: lookScales)
    func needsYouIsAmber(scale: Int) throws {
        let row = OrchestratorRowView(
            model: NavigatorOrchestrator(state: .needsYou, agent: "claude", status: .blocked, nowDoing: nil),
            inProgress: 0, selected: false, keyed: false
        )
        .padding(.horizontal, NavigatorGrid.edge)
        let rep = try bitmap(row, size: CGSize(width: 320, height: 40), scale: scale)
        var amber = 0
        var blue = 0
        // From the text column on: the status mark before it is `StatusGlyph`'s own.
        for x in (rep.pixelsWide * 3 / 10)..<rep.pixelsWide {
            for y in 0..<rep.pixelsHigh {
                guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                if c.redComponent > 0.7, c.greenComponent > 0.4, c.greenComponent < 0.8, c.blueComponent < 0.3 { amber += 1 }
                if c.blueComponent > 0.8, c.redComponent < 0.3 { blue += 1 }
            }
        }
        // Pixels, not points: the word measures about 236 at 1x and 1200 at 2x, so 20 holds at both.
        #expect(amber > 20, "no amber was drawn at \(scale)x")
        #expect(blue == 0, "the word is drawn in the accent: \(blue) px")
    }
}
