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

    /// The band's three stops at `phase` (0...1 through one pass): clear,
    /// opaque at the center, clear. The center travels from 0.35 before the
    /// first letter to 0.35 past the last, as the timeline's did, so the band
    /// enters and leaves rather than appearing at an edge.
    ///
    /// Not clamped to 0...1: `CAGradientLayer` extends the stops past the
    /// layer's edges, so a band half off the label draws as its visible half.
    /// `ShimmerAnimationTests` renders that.
    public static func locations(at phase: Double) -> [NSNumber] {
        let center = -0.35 + phase * 1.7
        return [center - halfWidth, center, center + halfWidth].map { NSNumber(value: $0) }
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
