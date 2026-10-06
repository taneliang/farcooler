import AppKit
import SwiftUI

/// A highlight sweeping across some text, moved by Core Animation (ov-382).
///
/// `WorkingRow` drew its sweep with a `TimelineView` at 30 frames a second,
/// which runs SwiftUI's update and redraws the window's display list on the
/// main thread for every frame. That sits under a transcript, and with a long
/// reply in view each frame cost tens of milliseconds: the main thread
/// stalled 50-170 ms several times a second for as long as an agent worked,
/// streaming or not, and stalls fell to none with the row gone. A layer
/// animation is run by the render server, so the main thread does nothing
/// per frame.
///
/// Drawn as `content` at full strength, seen through a moving band: the
/// text under it (the caller's, in `.secondary`) shows either side.
struct ShimmerBand<Content: View>: NSViewRepresentable {
    let content: Content

    func makeNSView(context: Context) -> ShimmerBandView<Content> {
        ShimmerBandView(content)
    }

    func updateNSView(_ view: ShimmerBandView<Content>, context: Context) {
        view.hosting.rootView = content
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: ShimmerBandView<Content>, context: Context) -> CGSize? {
        nsView.hosting.fittingSize
    }
}

/// The band: `content`, masked by a gradient whose stops move across it.
final class ShimmerBandView<Content: View>: NSView {
    let hosting: NSHostingView<Content>
    private let band = CAGradientLayer()

    init(_ content: Content) {
        hosting = NSHostingView(rootView: content)
        super.init(frame: .zero)
        wantsLayer = true
        addSubview(hosting)
        band.colors = [NSColor.clear.cgColor, NSColor.black.cgColor, NSColor.clear.cgColor]
        band.startPoint = CGPoint(x: 0, y: 0.5)
        band.endPoint = CGPoint(x: 1, y: 0.5)
        band.locations = ShimmerAnimation.start
        layer?.mask = band
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    override func layout() {
        super.layout()
        hosting.frame = bounds
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        band.frame = bounds
        CATransaction.commit()
    }

    /// The row is text to read, not to click; whatever is under it takes
    /// the clicks.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // Added on arrival, as `BreathingView` does: a layer's animations can
        // be dropped while it's out of a window.
        guard window != nil, band.animation(forKey: ShimmerAnimation.key) == nil else { return }
        band.add(ShimmerAnimation.make(), forKey: ShimmerAnimation.key)
    }
}

/// What `ShimmerBandView` adds to its mask: the band entering before the
/// first letter and leaving past the last, once every 1.1 s.
enum ShimmerAnimation {
    static let key = "shimmer"
    /// The band's stops, centered before the text and after it.
    static let start: [NSNumber] = [-0.65, -0.35, -0.05]
    static let end: [NSNumber] = [1.05, 1.35, 1.65]

    static func make() -> CABasicAnimation {
        let sweep = CABasicAnimation(keyPath: "locations")
        sweep.fromValue = start
        sweep.toValue = end
        sweep.duration = 1.1
        sweep.repeatCount = .infinity
        sweep.timingFunction = CAMediaTimingFunction(name: .linear)
        sweep.isRemovedOnCompletion = false
        return sweep
    }
}
