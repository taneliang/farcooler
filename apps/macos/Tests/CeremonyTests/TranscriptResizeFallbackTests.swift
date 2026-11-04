import AgentKit
import AppKit
import Testing

@testable import Far_Cooler

extension NativeSelectionTests.TranscriptScrollerTests {
    /// A missing row still uses the existing estimated aim, rather than
    /// requiring a direct frame as the retained-row restoration does.
    @MainActor
    @Test("A row let go during resize keeps the estimated restoration fallback")
    func aMissingRowKeepsItsEstimatedResizeFallback() async throws {
        let store = AgentRowStore(key: "resize-fallback-\(UUID())", cache: nil)
        store.apply(try await store.ledger.page(T.page(RealSession.rows(200, of: 200))))
        try #require(store.items.count >= 80)
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 720, height: 900))
        scroll.hasVerticalScroller = true
        scroll.scrollerStyle = .overlay
        scroll.automaticallyAdjustsContentInsets = false
        let doc = FlippedView(frame: NSRect(x: 0, y: 0, width: 720, height: 30_000))
        scroll.documentView = doc
        scroll.tile()
        let layout = TranscriptLayout(store: store, spacing: Spacing.inset)
        let driver = TranscriptScroller(layout: layout)
        layout.scroller = driver
        driver.attach(scroll)
        defer { driver.detach() }
        let built = (60..<68).map { store.items[$0].id }
        func report() {
            let width = scroll.contentView.bounds.width
            for (i, id) in built.enumerated() {
                layout.place(id, CGRect(
                    x: TranscriptLayout.padding, y: 12_000 + CGFloat(i) * (237 + Spacing.inset),
                    width: width - TranscriptLayout.padding * 2, height: 237), in: width)
            }
        }
        report()
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 12_000 + 0.3 * 237))
        driver.apply()
        let reference = try #require(layout.reading(scroll))
        try #require(reference.id == built[0])
        scroll.setFrameSize(NSSize(width: 600, height: 900))
        doc.setFrameSize(NSSize(width: 600, height: 30_000))
        scroll.tile()
        driver.apply()
        try #require(driver.restorePending)
        report()
        layout.drop(reference.id)
        driver.apply()
        try #require(!layout.has(reference.id))
        let expectedAim = try #require(layout.value(for: reference, in: scroll))
        let tick = NSSelectorFromString("ticked")
        try #require(driver.responds(to: tick))
        for _ in 0..<TranscriptScroller.widthFrames { _ = driver.perform(tick) }
        let aim = try #require(driver.aimed, "the missing row still restores through the estimated aim")
        #expect(abs(aim - expectedAim) < 1e-8)
        #expect(!driver.restorePending)
        #expect(LiveScroll.scrolls(scroll))
        for _ in 0..<(TranscriptScroller.settleFrames + 1) { _ = driver.perform(tick) }
        #expect(driver.aimed == nil)
        #expect(!driver.ticking)
        #expect(!LiveScroll.scrolls(scroll))
    }
}
