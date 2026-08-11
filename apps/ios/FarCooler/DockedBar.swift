import SwiftUI
import UIKit

/// A SwiftUI bar that is part of the KEYBOARD rather than part of the screen.
///
/// Why this exists at all: `scrollDismissesKeyboard(.interactively)` is defined
/// in terms of the keyboard's own rect — it is `UIScrollView`'s
/// `keyboardDismissMode = .interactive`, which knows nothing about a composer
/// resting above the keyboard. So the drag had to travel past the whole composer
/// before the keyboard registered it, and dismissing meant reaching down to the
/// keyboard first. An `inputAccessoryView` IS part of the keyboard, so a drag
/// that reaches the top of the bar starts the dismissal, and the bar tracks the
/// keyboard rather than being animated separately behind it.
///
/// The bar is hosted in a view controller that can be first responder and vends
/// it from `inputAccessoryView`. That is the long-standing chat-bar pattern, and
/// it is what keeps the bar docked at the bottom when nothing is being typed
/// into — an accessory attached only to the text field would vanish the moment
/// the field gave up focus, taking the composer off screen with it.
///
/// The SwiftUI content lives in a `UIHostingController` whose `rootView` is
/// re-assigned on every update pass. That is deliberately NOT the same as
/// rebuilding the view: the hosting controller keeps its own SwiftUI graph, so
/// `@State` inside the bar — the draft message, the cursor, the attachments —
/// survives, and only the values passed in from outside are refreshed.
struct DockedBar<Content: View>: UIViewControllerRepresentable {
    /// How tall the bar measured, reported back so the conversation above can
    /// leave room for it.
    ///
    /// Needed because a docked accessory with the keyboard DOWN posts no
    /// keyboard-frame notification — it is simply on screen — so nothing else
    /// knows it is there and the transcript ran underneath it.
    @Binding var height: CGFloat
    @ViewBuilder var content: () -> Content

    func makeUIViewController(context: Context) -> DockedBarController {
        DockedBarController(rootView: AnyView(content()))
    }

    func updateUIViewController(_ controller: DockedBarController, context: Context) {
        controller.onHeightChange = { measured in
            // Assigned only on a real change — `AccessoryHostView` already
            // filters — but guarded again here because writing SwiftUI state
            // from a UIKit layout pass is exactly where update loops start.
            guard abs(measured - height) > 0.5 else { return }
            height = measured
        }
        controller.update(rootView: AnyView(content()))
    }
}

/// How much of the screen the keyboard covers, accessory included.
///
/// Needed because SwiftUI's own keyboard avoidance is not enough once the
/// composer is an accessory: with the keyboard DOWN the bar is still docked and
/// still covering the bottom of the screen, and avoidance insets by nothing —
/// so the conversation ran underneath it. With the keyboard UP, avoidance insets
/// by the whole keyboard, accessory included, so simply adding the bar's height
/// on top would double-count it and leave a bar-sized gap.
///
/// One number from one notification covers both: the keyboard's reported frame
/// already includes the accessory, so its overlap with the screen IS the inset
/// in either state. The transcript opts out of automatic avoidance and uses
/// this instead.
@MainActor
final class KeyboardInset: ObservableObject {
    @Published private(set) var height: CGFloat = 0

    private var observers: [NSObjectProtocol] = []

    init() {
        let center = NotificationCenter.default
        // `willChangeFrame` rather than `willShow`: a docked accessory appearing
        // with no keyboard behind it, and the keyboard growing or shrinking for
        // a hardware keyboard or a language switch, are all frame changes and
        // none of them are a "show".
        for name in [
            UIResponder.keyboardWillChangeFrameNotification,
            UIResponder.keyboardDidChangeFrameNotification,
        ] {
            observers.append(
                center.addObserver(forName: name, object: nil, queue: .main) { [weak self] note in
                    MainActor.assumeIsolated { self?.apply(note) }
                })
        }
        observers.append(
            center.addObserver(
                forName: UIResponder.keyboardWillHideNotification, object: nil, queue: .main
            ) { [weak self] _ in
                // Hiding leaves the accessory docked, so this is not zero — the
                // next frame change reports what is left. Nothing is assumed
                // here beyond "the keyboard part is going away".
                MainActor.assumeIsolated { self?.height = 0 }
            })
    }

    deinit {
        observers.forEach(NotificationCenter.default.removeObserver)
    }

    private func apply(_ note: Notification) {
        guard
            let frame = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect,
            let screen = (note.object as? UIScreen) ?? UIApplication.shared.connectedScenes
                .compactMap({ ($0 as? UIWindowScene)?.screen }).first
        else { return }
        height = max(0, screen.bounds.height - frame.origin.y)
    }
}

/// Hosts the bar and keeps it docked.
final class DockedBarController: UIViewController {
    private let host: UIHostingController<AnyView>
    private lazy var bar = AccessoryHostView(host: host)

    init(rootView: AnyView) {
        host = UIHostingController(rootView: rootView)
        super.init(nibName: nil, bundle: nil)
        // The hosting controller draws its own background, which would sit as an
        // opaque slab behind a bar whose whole design is glass over the
        // conversation.
        host.view.backgroundColor = .clear
        // Deliberately NOT `addChild`.
        //
        // Containment says "this controller's view lives inside mine", and an
        // input accessory's does not: UIKit hands it to the keyboard's own
        // window. Claiming the relationship anyway crashed on launch — while
        // moving the accessory into that window UIKit walks the subtree
        // (`_associatedViewControllerForwardsAppearanceCallbacks:performHierarchyCheck:`),
        // finds a view whose controller has a parent that is not an ancestor,
        // and raises. The hosting controller is kept alive by this property
        // instead; only its appearance callbacks are given up, and an accessory
        // has no meaningful appearance transitions to forward.
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    /// This controller occupies no space. Everything it draws is in the
    /// accessory, which UIKit positions above the keyboard.
    override func loadView() {
        view = UIView(frame: .zero)
        view.isUserInteractionEnabled = false
    }

    override var canBecomeFirstResponder: Bool { true }
    override var inputAccessoryView: UIView? { bar }

    /// Called when the bar's measured height changes. Set by the representable.
    var onHeightChange: ((CGFloat) -> Void)? {
        get { bar.onHeightChange }
        set { bar.onHeightChange = newValue }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        // Docked from the start, so the composer is on screen before anybody
        // taps into it.
        becomeFirstResponder()
    }

    func update(rootView: AnyView) {
        host.rootView = rootView
        // The bar's height is content-driven — the field grows as you type, an
        // attachment strip appears, the mention list opens — and UIKit will not
        // re-measure an accessory on its own. `setNeedsLayout` rather than
        // measuring here: SwiftUI has not laid the new content out yet, so a
        // measurement taken now would be of the previous one.
        bar.setNeedsLayout()
    }
}

/// The accessory itself: a plain view whose height is whatever the SwiftUI
/// content needs.
///
/// `flexibleHeight` in the autoresizing mask is what tells UIKit this accessory
/// is willing to be measured rather than pinned to a fixed height; without it
/// the bar is drawn at whatever it happened to be born at and clips the moment
/// the field grows past one line.
final class AccessoryHostView: UIView {
    private let host: UIHostingController<AnyView>

    /// The height last measured, so `layoutSubviews` can tell a real change
    /// from being asked again at the same size.
    private var measured: CGFloat = 0

    /// Reported upward so the conversation can leave room. See `DockedBar`.
    var onHeightChange: ((CGFloat) -> Void)?

    init(host: UIHostingController<AnyView>) {
        self.host = host
        super.init(frame: .zero)
        autoresizingMask = .flexibleHeight
        backgroundColor = .clear
        // Springs and struts rather than constraints. An input accessory is
        // sized by UIKit through `intrinsicContentSize` and its own autoresizing
        // mask; adding Auto Layout that pins the content to all four edges puts
        // two systems in charge of the same height, and the one that loses
        // produces a bar clipped to zero or grown to the whole screen.
        host.view.frame = bounds
        host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        addSubview(host.view)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    /// Measured from the SwiftUI content, at the width the keyboard gives us.
    ///
    /// `noIntrinsicMetric` for width because an accessory is always the full
    /// width of the keyboard, and asking for one would fight that.
    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: fittedHeight())
    }

    private func fittedHeight() -> CGFloat {
        let width = bounds.width > 0 ? bounds.width : (window?.bounds.width ?? UIScreen.main.bounds.width)
        let fitted = host.sizeThatFits(in: CGSize(width: width, height: .greatestFiniteMagnitude))
        // Never zero: a bar measured before SwiftUI has laid anything out would
        // collapse the accessory, and a collapsed accessory takes the composer
        // off screen with no way to get it back.
        return max(fitted.height, 1)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        // The width is only known once UIKit has placed us, and the height
        // depends on it. Compared against the last MEASURED height rather than
        // against `bounds.height` — bounds is what UIKit chose, which need not
        // equal what the content wants, so comparing the two never converges and
        // invalidates forever.
        guard bounds.width > 0 else { return }
        let height = fittedHeight()
        guard abs(height - measured) > 0.5 else { return }
        measured = height
        invalidateIntrinsicContentSize()
        onHeightChange?(height)
    }
}
