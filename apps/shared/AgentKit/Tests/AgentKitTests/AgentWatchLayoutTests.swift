import Testing
@testable import AgentKit

/// A watched pane keeps the font's own size and scrolls, instead of shrinking to the phone's width (ov-976).
struct AgentWatchLayoutTests {
    @Test func aWidePaneIsDrawnWiderThanAPhone() {
        let size = WatchLayout.contentSize(columns: 120, rows: 40, cellWidth: 8, cellHeight: 16, padding: 6)
        #expect(size.width == 120 * 8 + 12 && size.height == 40 * 16 + 12)
        // 320 pt is the narrowest phone: the pane is three times that, so it scrolls.
        #expect(size.width > 320)
    }
}
