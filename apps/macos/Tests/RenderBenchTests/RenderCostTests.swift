import AppKit
import Foundation
import Testing

@testable import Far_Cooler

/// What one frame of terminal actually costs.
///
/// Not a correctness suite. It exists so that "the renderer is fast enough" is
/// a number somebody measured rather than a thing somebody believed, and so an
/// ablation can say WHICH part of the frame the number is in. Skipped unless
/// `FARCOOLER_RENDER_BENCH` is set, because a benchmark that runs on every
/// `swift test` is a slow suite that tells you nothing new.
@MainActor
struct RenderCostTests {
    static var enabled: Bool { ProcessInfo.processInfo.environment["FARCOOLER_RENDER_BENCH"] != nil }

    /// A grid of the size a real pane is, filled the way real output fills it.
    private func view(columns: Int, rows: Int, fill: Fill) -> TerminalRenderView {
        let v = TerminalRenderView()
        let cell = v.metrics
        v.frame = NSRect(
            x: 0, y: 0,
            width: CGFloat(columns) * cell.cellWidth + cell.paddingLeft * 2,
            height: CGFloat(rows) * cell.cellHeight + cell.paddingTop * 2)
        v.setPaneGrid(PaneGrid(columns: columns, rows: rows))
        v.feed(Array(fill.bytes(columns: columns, rows: rows).utf8))
        return v
    }

    enum Fill {
        /// Every cell blank. The floor: backgrounds, cursor, nothing else.
        case blank
        /// Plain uncoloured text in every cell.
        case text
        /// Text plus an SGR change every eight cells, which is what an agent's
        /// syntax-highlighted output looks like.
        case colored

        func bytes(columns: Int, rows: Int) -> String {
            var out = "\u{1b}[H"
            for row in 0..<rows {
                out += "\u{1b}[\(row + 1);1H"
                switch self {
                case .blank:
                    out += String(repeating: " ", count: columns)
                case .text:
                    out += String(
                        repeating: "the quick brown fox jumps over the lazy dog ",
                        count: columns / 44 + 1
                    ).prefix(columns).description
                case .colored:
                    var column = 0
                    while column < columns {
                        let run = min(8, columns - column)
                        out += "\u{1b}[38;5;\((column + row) % 200 + 16)m"
                        out += String("abcdefgh".prefix(run))
                        column += run
                    }
                }
            }
            return out
        }
    }

    /// Draw the view `count` times into a bitmap and return milliseconds per frame.
    private func msPerFrame(_ v: TerminalRenderView, count: Int) -> Double {
        let scale = 2.0
        let w = Int(v.bounds.width * scale)
        let h = Int(v.bounds.height * scale)
        let space = CGColorSpaceCreateDeviceRGB()
        guard
            let ctx = CGContext(
                data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                space: space, bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue)
        else { return .nan }
        ctx.scaleBy(x: scale, y: scale)

        let graphics = NSGraphicsContext(cgContext: ctx, flipped: true)
        // Warm the caches the first frame would pay for.
        NSGraphicsContext.current = graphics
        v.draw(v.bounds)

        // Best of several rounds, not the mean. This machine runs other builds,
        // and a loaded one inflates every number by the same multiple — which
        // is how a cache that changed nothing can look like a 40% win. The
        // fastest round is the one that was least interfered with.
        var best = Double.infinity
        for _ in 0..<7 {
            let start = CFAbsoluteTimeGetCurrent()
            for _ in 0..<count {
                NSGraphicsContext.current = graphics
                v.draw(v.bounds)
            }
            best = min(best, (CFAbsoluteTimeGetCurrent() - start) / Double(count) * 1000)
        }
        NSGraphicsContext.current = nil
        return best
    }

    @Test("One frame, ablated: blank grid, plain text, coloured text")
    func theFrameCost() {
        // Returns rather than fails when unset. There is no skip in
        // swift-testing, and a benchmark that reddens every ordinary `swift
        // test` would be deleted within a week.
        guard Self.enabled else { return }
        let grids = [(80, 24), (120, 40), (190, 50), (240, 70)]
        for (columns, rows) in grids {
            var line = "\(columns)x\(rows)  "
            for (name, fill) in [("blank", Fill.blank), ("text", .text), ("colored", .colored)] {
                let v = view(columns: columns, rows: rows, fill: fill)
                let ms = msPerFrame(v, count: 60)
                line += String(format: "%@ %.2f ms (%.0f fps)  ", name, ms, 1000 / ms)
            }
            print(line)
        }
    }
}
