import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// What the eye sees between the title bar's items (ov-417): the switcher's
/// chevron, then the orchestrator's mark, are not touching and don't read as
/// a pair with a second chevron.
///
/// Measured on pixels of the real titled window, because the gap that
/// mattered was ink, not frames: the status area's content is wider than its
/// ring form, and a fixed-width item centers what overflows it, so the mark
/// sat against the switcher while every frame said 15 pt apart.
@MainActor
@Suite(.serialized)
struct TitleBarGapTests {
    typealias Harness = TitleBarHarness

    /// The toolbar's gap between items (`TitleStatus.available`'s).
    static let itemGap: CGFloat = 8

    /// Runs of columns with ink, left to right, in points, across the
    /// toolbar's band from `from` to `to`, runs less than `merging` pt apart merged
    /// (a glyph's own parts). Ink is a pixel that differs from the one at the
    /// top of its column, which is the bar's background.
    static func inkRuns(_ rep: NSBitmapImageRep, from: CGFloat, to: CGFloat, merging: CGFloat = 3) -> [ClosedRange<CGFloat>] {
        let scale = CGFloat(rep.pixelsWide) / rep.size.width
        func differs(_ a: NSColor, _ b: NSColor) -> Bool {
            abs(a.redComponent - b.redComponent) + abs(a.greenComponent - b.greenComponent)
                + abs(a.blueComponent - b.blueComponent) > 0.25
        }
        var runs: [ClosedRange<CGFloat>] = []
        for px in Int(from * scale)..<Int(to * scale) {
            let background = rep.colorAt(x: px, y: 2)!.usingColorSpace(.sRGB)!
            // The middle of the 52 pt band, 40 pt of it.
            let inked = (Int(6 * scale)..<Int(46 * scale)).contains { y in
                differs(rep.colorAt(x: px, y: y)!.usingColorSpace(.sRGB)!, background)
            }
            guard inked else { continue }
            let x = CGFloat(px) / scale
            if let last = runs.last, x - last.upperBound < merging {
                runs[runs.count - 1] = last.lowerBound...x
            } else {
                runs.append(x...x)
            }
        }
        return runs
    }

    @Test("The mark clears the switcher's chevron by the toolbar's gap", arguments: [CGFloat(640), 720, 900])
    func gapAfterTheTitle(width: CGFloat) async throws {
        let words = Harness.Words()
        words.title = "Billing and Subscriptions Platform"
        words.repository = "demo"
        words.orchestrator = .none
        let window = try await Harness.window(Harness.Root(words: words, content: Color.clear), width: width)
        defer { window.close() }
        let status = try #require(Harness.status(in: window))
        let switcher = try #require(Harness.switcher(in: window))
        let view = try #require(window.contentView?.superview)
        let rep = try #require(view.lookBitmap(scale: 2))
        // From inside the switcher's chevron to the end of the area.
        let runs = Self.inkRuns(rep, from: switcher.maxX - 12, to: status.frame.maxX, merging: 1)
        let chevron = try #require(runs.first, "no ink at the switcher's end at \(width)")
        let next = try #require(runs.dropFirst().first, "no mark after the switcher at \(width)")
        // The chevron's ink is its own run, ending where the switcher does;
        // a mark touching it is the same run.
        #expect(chevron.upperBound <= switcher.maxX + 1, "the mark touches the title's chevron at \(width): \(runs)")
        #expect(next.lowerBound - chevron.upperBound >= Self.itemGap, "at \(width): \(runs)")
    }

    @Test("The orchestrator's chevron goes with its words, so the ring has only the mark")
    func chevronGoesWithTheWords() {
        #expect(!TitleStatus.showsMenuIndicator(.ring))
        for form in TitleStatus.Form.allCases where form >= .short {
            #expect(TitleStatus.showsMenuIndicator(form), "\(form)")
        }
    }

    @Test("The ring form holds its content inside its width, whatever the count")
    func ringHoldsItsContent() async throws {
        let words = Harness.Words()
        words.title = "Billing and Subscriptions Platform"
        words.repository = "demo"
        words.orchestrator = .none
        words.needYou = 142
        let window = try await Harness.window(Harness.Root(words: words, content: Color.clear), width: 640)
        defer { window.close() }
        let status = try #require(Harness.status(in: window))
        let switcher = try #require(Harness.switcher(in: window))
        let rep = try #require(window.contentView?.superview?.lookBitmap(scale: 2))
        let runs = Self.inkRuns(rep, from: switcher.maxX - 12, to: status.frame.maxX + 40)
        let mark = try #require(runs.dropFirst().first)
        #expect(mark.lowerBound - switcher.maxX >= Self.itemGap, "\(runs)")
        #expect(runs.last!.upperBound <= status.frame.maxX + 1, "content runs past the area: \(runs) in \(status.frame)")
    }
}
