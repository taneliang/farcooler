import QuartzCore

/// The sweep across "Working…", as a Core Animation animation of a gradient
/// mask's stops (ov-382). One definition for the Mac's `ShimmerBandView` and
/// the iPhone's, so the two sweep alike.
///
/// A layer animation because the render server runs it: the main thread does
/// nothing per frame. A `TimelineView` doing the same work ran SwiftUI's
/// update every frame and, beside a long reply, stalled the Mac's main
/// thread 50-170 ms at a time for the whole of a turn.
public enum ShimmerAnimation {
    public static let key = "shimmer"
    /// One pass, in seconds: the timeline's period, kept.
    public static let period: CFTimeInterval = 1.1
    /// Half the band's width, as a fraction of the label's.
    static let halfWidth = 0.3

    /// How far the band reaches, in the label's widths: from wholly before
    /// the first letter to wholly past the last.
    static let reach = (from: -0.35 - halfWidth, to: 1.35 + halfWidth)

    /// The band's three stops at `phase` (0...1 through one pass): clear,
    /// opaque at the center, clear. The center travels from 0.35 of the
    /// label's width before its first letter to 0.35 past its last, as the
    /// timeline's did, so the band enters and leaves rather than appearing at
    /// an edge.
    ///
    /// In the band layer's own 0...1, which spans `reach` (`bandFrame`).
    /// `CAGradientLayer` ignores stops outside 0...1 and spaces the colors
    /// evenly instead, so stops in the label's own units, which run from
    /// -0.65 to 1.65, drew a band parked mid-label for most of each pass
    /// (ov-382 review). `ShimmerAnimationTests` renders it.
    public static func locations(at phase: Double) -> [NSNumber] {
        let center = -0.35 + phase * 1.7
        let span = reach.to - reach.from
        return [center - halfWidth, center, center + halfWidth].map { NSNumber(value: ($0 - reach.from) / span) }
    }

    /// The band layer's frame for a label laid out in `bounds`: as tall,
    /// and wide enough to reach from before the label to past it. A mask
    /// only counts where it overlaps, so the part outside does nothing.
    public static func bandFrame(in bounds: CGRect) -> CGRect {
        CGRect(
            x: bounds.minX + bounds.width * reach.from, y: bounds.minY,
            width: bounds.width * (reach.to - reach.from), height: bounds.height)
    }

    /// Where the band rests between passes, and where a still label leaves
    /// it: wholly before the text, so nothing of it shows.
    public static var start: [NSNumber] { locations(at: 0) }

    public static func make() -> CABasicAnimation {
        let sweep = CABasicAnimation(keyPath: "locations")
        sweep.fromValue = locations(at: 0)
        sweep.toValue = locations(at: 1)
        sweep.duration = period
        sweep.repeatCount = .infinity
        sweep.timingFunction = CAMediaTimingFunction(name: .linear)
        sweep.isRemovedOnCompletion = false
        return sweep
    }
}
