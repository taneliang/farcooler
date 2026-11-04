import AgentKit
import AppKit
import Testing

@testable import Far_Cooler

extension NativeSelectionTests.TranscriptScrollerTests {
    /// Supplied geometry isolates TRACE-1's retained-row branch: the saved
    /// row survives, but its saved point moves 16.1 pt below the clip. Drive
    /// the real display-link selector synchronously, without a window or
    /// run-loop wait; this is a decision-seam check, not a mounted resize.
    @MainActor
    @Test("A retained row returns to its saved within-row point after a width hold")
    func aRetainedRowRestoresItsSavedShare() async throws {
        let store = AgentRowStore(key: "resize-anchor-\(UUID())", cache: nil)
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
        let rowY: CGFloat = 12_000
        let rowHeight: CGFloat = 237
        let share: CGFloat = 0.3
        let shift: CGFloat = 16.1
        func report(shift: CGFloat) {
            let width = scroll.contentView.bounds.width
            for (i, id) in built.enumerated() {
                layout.place(id, CGRect(
                    x: TranscriptLayout.padding,
                    y: rowY + shift + CGFloat(i) * (rowHeight + Spacing.inset),
                    width: width - TranscriptLayout.padding * 2, height: rowHeight), in: width)
            }
        }
        report(shift: 0)
        scroll.contentView.scroll(to: NSPoint(x: 0, y: rowY + share * rowHeight))
        driver.apply()
        let reference = try #require(layout.reading(scroll))
        try #require(reference.id == built[0])
        try #require(abs(reference.share - share) < 1e-8)
        let knob = try #require(layout.knob(scroll))
        try #require(knob.value > 0.001 && knob.value < 0.999, "capture is away from either end")
        try #require(!LiveScroll.scrolls(scroll))

        // Capture from coherent old frames at the width change, before
        // reporting the translated row. This excludes late-capture error
        // from this conditional test of the retained-row restore branch.
        scroll.setFrameSize(NSSize(width: 600, height: 900))
        doc.setFrameSize(NSSize(width: 600, height: 30_000))
        scroll.tile()
        driver.apply()
        try #require(driver.restorePending, "the width change must save a reading")
        try #require(layout.reading(scroll) == reference, "capture precedes the supplied translation")
        report(shift: shift)
        driver.apply()
        try #require(layout.has(reference.id), "the saved row remains built")
        try #require(layout.reading(scroll)?.id == reference.id, "the ID alone already holds")
        let displaced = try #require(layout.origin(for: reference, in: scroll))
        try #require(abs(displaced - scroll.contentView.bounds.minY - shift) < 0.01,
                     "the setup must introduce the observed within-row point displacement")

        // Use the existing Objective-C entry called by CADisplayLink, not a
        // copied countdown/restore predicate or a new product-only test hook.
        // No await after attaching: native callbacks cannot interleave with
        // this supplied sequence on the main actor.
        let tick = NSSelectorFromString("ticked")
        try #require(driver.responds(to: tick))
        for _ in 0..<(TranscriptScroller.widthFrames + TranscriptScroller.settleFrames + 1) {
            _ = driver.perform(tick)
        }
        try #require(!driver.restorePending && !driver.ticking, "the production width hold and aim have ended")
        let restored = try #require(layout.reading(scroll))
        #expect(restored.id == reference.id)
        // Oracle from the supplied geometry, independent of estimated sums
        // or normalized knob value. Half a point allows coordinate rounding.
        let expectedY = rowY + shift + share * rowHeight
        #expect(abs(scroll.contentView.bounds.minY - expectedY) < 0.5,
                "the retained row must return to its saved point, not merely keep its ID")
        #expect(abs(restored.share - reference.share) * rowHeight < 0.5,
                "the saved within-row point must hold through the resize")
    }
}
