import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// What the title bar's Needs You item says and how it's colored (ov-91,
/// quieted in ov-105).
@MainActor
struct NeedsYouToolbarTests {
    @Test("The tooltip names the count, and says nothing of it when nothing waits")
    func tooltipNamesTheCount() {
        #expect(NeedsYouToolbar.tooltip(count: 0) == "Needs You")
        #expect(NeedsYouToolbar.tooltip(count: 1) == "1 thing needs you")
        #expect(NeedsYouToolbar.tooltip(count: 12) == "12 things need you")
    }

    @Test("The count is text beside the tray: none at zero, capped at 99+")
    func countText() {
        #expect(NeedsYouToolbar.countText(count: 0) == nil)
        #expect(NeedsYouToolbar.countText(count: 3) == "3")
        #expect(NeedsYouToolbar.countText(count: 120) == "99+")
    }

    /// The owner (ov-291): the accent count was unreadable. The label's own
    /// color, drawn: no pixel of the button is the tint, which is set to red.
    @Test("The count is drawn in the toolbar's label color, never the accent")
    func countIsPrimary() throws {
        #expect(NeedsYouToolbar.countInk == Color.primary)
        let button = NeedsYouToolbarButton(count: 3, selected: false, onSelect: {}).buttonStyle(.plain).tint(.red)
        let host = NSHostingView(rootView: button.frame(width: 90, height: 36))
        host.frame = CGRect(x: 0, y: 0, width: 90, height: 36)
        host.appearance = NSAppearance(named: .aqua)
        host.layoutSubtreeIfNeeded()
        let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        var red = 0
        for x in 0..<rep.pixelsWide {
            for y in 0..<rep.pixelsHigh {
                guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                if c.alphaComponent > 0.5, c.redComponent > 0.7, c.greenComponent < 0.35, c.blueComponent < 0.35 { red += 1 }
            }
        }
        #expect(red == 0, "\(red) accent pixels in the button")
    }

    @Test("VoiceOver hears how many are waiting")
    func accessibility() {
        #expect(NeedsYouToolbar.accessibilityLabel(count: 0) == "Needs You, nothing waiting")
        #expect(NeedsYouToolbar.accessibilityLabel(count: 1) == "Needs You, 1 waiting")
        #expect(NeedsYouToolbar.accessibilityLabel(count: 3) == "Needs You, 3 waiting")
    }
}
