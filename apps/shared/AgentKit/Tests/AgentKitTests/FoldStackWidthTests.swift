#if os(macOS)
import SwiftUI
import Testing
@testable import AgentKit

/// A bubble hugs a short message (ov-809). `FoldStack` took the whole width
/// it was offered, so every message bubble was as wide as its cap, whatever
/// it said. Run under macOS, where `ImageRenderer` lays a view out headlessly;
/// the iPhone's UI test (`testYourBubbleHugsItsWords`) checks the same on a
/// device, with the text view that draws it there.
@MainActor
@Suite struct FoldStackWidthTests {
    /// What a layout was asked, and answered by, in one pass.
    final class Seen { var size: CGSize = .zero }

    /// Proposes `width` to its one child, as `TrailingMeasure` proposes its
    /// cap, and notes the size the child answers with.
    struct Probe: Layout {
        let width: CGFloat
        let seen: Seen
        func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
            guard let child = subviews.first else { return .zero }
            let size = child.sizeThatFits(ProposedViewSize(width: width, height: nil))
            seen.size = size
            return size
        }
        func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
            subviews.first?.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
        }
    }

    /// The fold of a message as `FoldedText` builds it, offered `width`.
    func fitted(_ text: String, offered width: CGFloat, fold: Fold = .lines) -> CGSize {
        let seen = Seen()
        let view = Probe(width: width, seen: seen) {
            FoldStack(fold: fold, open: false) {
                Text(verbatim: text).fixedSize(horizontal: false, vertical: true)
                Button("Show More") {}
                Text(verbatim: text).fixedSize(horizontal: false, vertical: true).hidden()
            }
        }
        let renderer = ImageRenderer(content: view)
        _ = renderer.cgImage
        return seen.size
    }

    /// The width `text` takes on its own, on one line.
    func natural(_ text: String) -> CGFloat {
        let seen = Seen()
        let renderer = ImageRenderer(content: Probe(width: 10_000, seen: seen) { Text(verbatim: text) })
        _ = renderer.cgImage
        return seen.size.width
    }

    /// The caps of a bubble in the column of a 375 and a 393 pt phone.
    static let caps: [CGFloat] = [(375 - 32) * MessageBubble.widest, (393 - 32) * MessageBubble.widest]

    @Test(arguments: caps)
    func aWordTakesItsOwnWidth(cap: CGFloat) {
        let width = fitted("Thanks.", offered: cap).width
        #expect(abs(width - natural("Thanks.")) <= 1, "a word at a cap of \(cap) took \(width)")
        #expect(width < cap / 2)
    }

    @Test(arguments: caps)
    func aLineTakesItsOwnWidth(cap: CGFloat) {
        let line = "Is the web interface ready btw"
        let width = fitted(line, offered: cap).width
        #expect(abs(width - natural(line)) <= 1, "a line at a cap of \(cap) took \(width), not \(natural(line))")
        #expect(width < cap, "a line that fits is as wide as its cap, \(cap)")
    }

    @Test(arguments: caps)
    func severalLinesTakeTheLongest(cap: CGFloat) {
        let text = "Two things:\nfirst the lexer\nthen the parser, please"
        let width = fitted(text, offered: cap).width
        #expect(abs(width - natural("then the parser, please")) <= 1, "took \(width) at a cap of \(cap)")
    }

    @Test(arguments: caps)
    func aLongMessageFillsItsCap(cap: CGFloat) {
        let text = String(repeating: "Fold this where it wraps past six lines. ", count: 12)
        #expect(fitted(text, offered: cap).width == cap)
    }

    /// A report's blocks still fill the column they're given.
    @Test(arguments: caps)
    func aReportFillsItsColumn(cap: CGFloat) {
        #expect(fitted("Done.", offered: cap, fold: .height(lines: 4)).width == cap)
    }
}
#endif
