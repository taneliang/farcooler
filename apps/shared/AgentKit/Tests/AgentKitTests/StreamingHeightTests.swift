// ov-382 review: settling a streaming reply mustn't move what's below a
// reader. Streaming, the last prose run's paragraphs are separate `Text`s
// with the run's 16 pt gap between them; settled, they're one `Text` with a
// 9 pt blank line between. Each `Text` rounds its own height to the pixel
// grid, so the two can't be equal to the point: the test holds the drift to
// a fraction of a point per paragraph.
//
// Measured, 30 paragraphs (29 boundaries): with the gap as padding, 3-3.5 pt
// in all at 2x and 1-3 pt at 1x; with each split paragraph drawn after the
// run's own blank line instead (the same mechanism as settled), 7-9 pt at 2x
// and 11-14 pt at 1x, as each piece then rounds a line of its own. So the
// padding stays.
#if os(macOS)
import AppKit
import SwiftUI
import Testing
@testable import AgentKit

@MainActor
struct StreamingHeightTests {
    /// Thirty paragraphs of one to three sentences: one prose run.
    static let reply: String = (0..<30).map { p in
        (0...(p % 3)).map { s in "Paragraph \(p) says something in sentence \(s), at a reasonable length for a reply." }
            .joined(separator: " ")
    }.joined(separator: "\n\n")

    /// The reply's height at `width`, laid out in a window, so at the
    /// screen's scale.
    static func height(streaming: Bool, width: CGFloat) -> (CGFloat, CGFloat) {
        let host = NSHostingView(rootView: AgentReplyText(text: reply, trailingClearance: 32, streaming: streaming).frame(width: width))
        let window = NSWindow(
            contentRect: NSRect(x: -9000, y: -9000, width: width, height: 400), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        window.contentView = host
        return (host.fittingSize.height, window.backingScaleFactor)
    }

    @Test(arguments: [320.0, 520.0, 760.0])
    func settlingKeepsTheHeight(width: Double) {
        let (streaming, scale) = Self.height(streaming: true, width: width)
        let (settled, _) = Self.height(streaming: false, width: width)
        print(String(format: "ov-382 height at %.0f, %.0fx: streaming %.1f, settled %.1f", width, scale, streaming, settled))
        // 6 pt in all: 2.5 pt over the 3.5 measured at 2x, so a different
        // display scale or font metric on CI doesn't tip it, and 1 pt under
        // the 7 the blank-line variant drifted at its best.
        #expect(abs(streaming - settled) <= 6, "streaming \(streaming) against settled \(settled) at \(width)")
    }
}
#endif
