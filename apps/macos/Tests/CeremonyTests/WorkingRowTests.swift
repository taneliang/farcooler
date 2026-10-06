import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// `WorkingRow`'s sweep runs on a layer (ov-382): the row is the label's size,
/// draws its words, and once in a window its band carries the animation, so
/// nothing on the main thread moves it.
@MainActor
struct WorkingRowTests {
    /// The band views under `view`.
    static func bands(in view: NSView) -> [NSView] {
        let mine = String(describing: type(of: view)).hasPrefix("ShimmerBandView") ? [view] : []
        return mine + view.subviews.flatMap(bands)
    }

    @Test func theRowIsTheLabelAndItsBandSweepsOnALayer() throws {
        let row = NSHostingView(rootView: WorkingRow())
        let label = NSHostingView(rootView: Text("Working…").font(.callout))
        let size = row.fittingSize
        #expect(abs(size.width - label.fittingSize.width) < 1, "\(size) against \(label.fittingSize)")
        #expect(abs(size.height - label.fittingSize.height) < 1, "\(size) against \(label.fittingSize)")

        let window = NSWindow(
            contentRect: NSRect(x: -9000, y: -9000, width: 200, height: 40), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        row.frame = NSRect(origin: .zero, size: size)
        window.contentView = row
        row.layoutSubtreeIfNeeded()

        let band = try #require(Self.bands(in: row).first, "no ShimmerBandView in the row")
        #expect(band.layer?.mask?.animation(forKey: ShimmerAnimation.key) != nil)

        // The words are drawn: the label under the band, in `.secondary`.
        let rep = try #require(row.bitmapImageRepForCachingDisplay(in: row.bounds))
        row.cacheDisplay(in: row.bounds, to: rep)
        var inked = 0
        for x in 0..<rep.pixelsWide {
            for y in 0..<rep.pixelsHigh where (rep.colorAt(x: x, y: y)?.alphaComponent ?? 0) > 0.2 {
                inked += 1
            }
        }
        #expect(inked > 20, "the row drew \(inked) inked pixels")
    }

    /// Under Reduce Motion, no sweep: the still label alone. Through
    /// `workingRowStill`, which is what a test can set.
    @Test func reduceMotionLeavesTheLabelStill() throws {
        let row = NSHostingView(rootView: WorkingRow().environment(\.workingRowStill, true))
        let window = NSWindow(
            contentRect: NSRect(x: -9000, y: -9000, width: 200, height: 40), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        row.frame = NSRect(origin: .zero, size: row.fittingSize)
        window.contentView = row
        row.layoutSubtreeIfNeeded()
        #expect(Self.bands(in: row).isEmpty, "a sweep under Reduce Motion")
    }
}
