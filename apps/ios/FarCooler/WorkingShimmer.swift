import SwiftUI
import UIKit

/// A highlight sweeping across some text, moved by Core Animation: the Mac's
/// `ShimmerBand`, ported (ov-382).
///
/// `WorkingRow` drew its sweep from a `TimelineView(.animation)`, which runs
/// SwiftUI's update on the main thread at the display's rate, 120 times a
/// second on a ProMotion phone, for the whole of a turn. A layer animation is
/// run by the render server, so the main thread does nothing per frame.
///
/// Drawn as `content` at full strength, seen through a moving band: the
/// text under it (the caller's, in `.secondary`) shows either side.
struct ShimmerBand<Content: View>: UIViewRepresentable {
    let content: Content
    /// Told once the sweep is on its layer: what `WorkingRow`'s debug probe
    /// reports, so a UI test can see where the motion comes from.
    var onSweeping: (() -> Void)? = nil

    func makeUIView(context: Context) -> ShimmerBandView<Content> {
        ShimmerBandView(content)
    }

    func updateUIView(_ view: ShimmerBandView<Content>, context: Context) {
        view.hosting.rootView = content
        view.onSweeping = onSweeping
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: ShimmerBandView<Content>, context: Context) -> CGSize? {
        uiView.hosting.sizeThatFits(in: CGSize(width: proposal.width ?? .greatestFiniteMagnitude, height: .greatestFiniteMagnitude))
    }
}

/// The band: `content`, masked by a gradient whose stops move across it.
final class ShimmerBandView<Content: View>: UIView {
    let hosting: UIHostingController<Content>
    let band = CAGradientLayer()
    var onSweeping: (() -> Void)?

    init(_ content: Content) {
        hosting = UIHostingController(rootView: content)
        super.init(frame: .zero)
        hosting.view.backgroundColor = .clear
        isUserInteractionEnabled = false
        addSubview(hosting.view)
        band.colors = [UIColor.clear.cgColor, UIColor.black.cgColor, UIColor.clear.cgColor]
        band.startPoint = CGPoint(x: 0, y: 0.5)
        band.endPoint = CGPoint(x: 1, y: 0.5)
        band.locations = ShimmerAnimation.start
        layer.mask = band
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override func layoutSubviews() {
        super.layoutSubviews()
        hosting.view.frame = bounds
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        band.frame = ShimmerAnimation.bandFrame(in: bounds)
        CATransaction.commit()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        // Added on arrival: a layer's animations are dropped while it's out
        // of a window, and when the app goes to the background.
        guard window != nil, band.animation(forKey: ShimmerAnimation.key) == nil else { return }
        band.add(ShimmerAnimation.make(), forKey: ShimmerAnimation.key)
        // Next turn, not during the window move: it writes SwiftUI state.
        DispatchQueue.main.async { [weak self] in
            guard let self, self.band.animation(forKey: ShimmerAnimation.key) != nil else { return }
            self.onSweeping?()
        }
    }
}
