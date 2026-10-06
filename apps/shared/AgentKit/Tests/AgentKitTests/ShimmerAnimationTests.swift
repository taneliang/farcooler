// ov-382 review: the Working… sweep moves, and its band is where the old
// timeline put it. The animation interpolates the gradient's stops linearly
// from `locations(at: 0)` to `locations(at: 1)`; these render the mask with
// the stops it has at a phase and read the band back out of the pixels,
// at the edges too. With the stops in the label's own units, -0.65 to 1.65,
// `CAGradientLayer` ignored them: the band rendered the same at phase 0.3
// as at 0.7, peaked mid-label, and didn't move.
#if os(macOS)
import AppKit
import QuartzCore
import Testing
@testable import AgentKit

struct ShimmerAnimationTests {
    static let width = 200

    /// The mask's alpha across a label `width` wide at `phase`, one value a
    /// column: the band layer where the views put it, rendered in the
    /// label's bounds.
    static func alphas(at phase: Double) -> [Double] {
        let label = CALayer()
        label.frame = CGRect(x: 0, y: 0, width: width, height: 4)
        let band = CAGradientLayer()
        band.colors = [NSColor.clear.cgColor, NSColor.black.cgColor, NSColor.clear.cgColor]
        band.startPoint = CGPoint(x: 0, y: 0.5)
        band.endPoint = CGPoint(x: 1, y: 0.5)
        band.locations = ShimmerAnimation.locations(at: phase)
        band.frame = ShimmerAnimation.bandFrame(in: label.bounds)
        label.addSublayer(band)
        let context = CGContext(
            data: nil, width: width, height: 4, bitsPerComponent: 8, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        label.render(in: context)
        let pixels = context.data!.assumingMemoryBound(to: UInt8.self)
        return (0..<width).map { Double(pixels[(width * 2 + $0) * 4 + 3]) / 255 }
    }

    /// Every stop the animation passes through is one `CAGradientLayer`
    /// honors.
    @Test func everyStopIsInRange() {
        for phase in stride(from: 0.0, through: 1.0, by: 0.05) {
            #expect(ShimmerAnimation.locations(at: phase).allSatisfy { (0...1).contains($0.doubleValue) })
        }
    }

    /// The old timeline's center at `phase`, as a column.
    static func oldCenter(_ phase: Double) -> Double { (-0.35 + phase * 1.7) * Double(width) }

    @Test(arguments: [0.3, 0.5, 0.7])
    func theBandIsWhereTheTimelinePutIt(phase: Double) {
        let alpha = Self.alphas(at: phase)
        let peak = alpha.indices.max { alpha[$0] < alpha[$1] }!
        #expect(abs(Double(peak) - Self.oldCenter(phase)) <= 2, "the band peaks at \(peak), not \(Self.oldCenter(phase))")
        #expect(alpha[peak] > 0.9)
        // Clear outside the band, a band's half-width either side.
        #expect(alpha[0] < 0.05 || Self.oldCenter(phase) - 0.3 * Double(Self.width) < 0)
    }

    @Test func theSweepMoves() {
        #expect(Self.alphas(at: 0.3) != Self.alphas(at: 0.7))
    }

    /// Stops outside 0...1 extend past the edge: a band centered before the
    /// first letter shows its trailing half, and at rest none of it.
    @Test func aBandOffTheEdgeShowsOnlyItsVisibleHalf() {
        // Phase 0.1: center at -0.18 of the width, so at the left edge the
        // band is 0.18/0.3 of the way down its slope: alpha 0.4.
        let entering = Self.alphas(at: 0.1)
        #expect(abs(entering[0] - 0.4) < 0.06, "at the edge \(entering[0])")
        #expect(entering[Self.width - 1] < 0.05)
        // At rest, before the text: nothing shows.
        #expect(Self.alphas(at: 0).allSatisfy { $0 < 0.05 })
    }
}
#endif
