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
    /// Whether this bar should be docked at all.
    ///
    /// An input accessory lives in the KEYBOARD's window, not in the view that
    /// vends it, so hiding that view does nothing to it. `ShellPaneTrack` keeps every
    /// visited pane mounted, so without this every chat pane ever opened went on
    /// holding first responder and went on drawing its composer — over the
    /// terminal, and over a changes pane that has no composer at all.
    var isActive: Bool
    /// Whose inset hears this bar's size reports. See `KeyboardInset.scope`.
    var scope: AccessoryScope? = nil
    @ViewBuilder var content: () -> Content

    func makeUIViewController(context: Context) -> DockedBarController {
        let controller = DockedBarController(rootView: AnyView(content()))
        controller.scope = scope
        return controller
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
        // After the content, so a bar becoming active docks with the right
        // thing in it rather than with whatever it last held.
        controller.setActive(isActive)
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

    /// The rules the number is published by. See `KeyboardCover`.
    private var cover = KeyboardCover()

    private var observers: [NSObjectProtocol] = []

    /// The composer this inset listens to, or nil for any docked composer.
    ///
    /// A chat's own inset names its own, so a second composer on screen
    /// can't move it (ov-386). The shell's inset is the one that takes them
    /// all: it cancels what the framework applies for whichever is up.
    let scope: AccessoryScope?

    /// Waits out a hide whose closing frame doesn't come.
    private var hideTimeout: Task<Void, Never>?

    init(scope: AccessoryScope? = nil) {
        self.scope = scope
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
            AccessoryCoverChannel.observe(scope: scope) { [weak self] cover, screenHeight in
                MainActor.assumeIsolated { self?.apply(accessoryCover: cover, screenHeight: screenHeight) }
            })
        observers.append(
            center.addObserver(
                forName: UIResponder.keyboardWillHideNotification, object: nil, queue: .main
            ) { [weak self] _ in
                // Hiding leaves the accessory docked, so this is not zero — the
                // next valid frame change reports what is left. Nothing is
                // assumed here beyond "the keyboard part is going away".
                MainActor.assumeIsolated { self?.willHide() }
            })
    }

    deinit {
        observers.forEach(NotificationCenter.default.removeObserver)
        hideTimeout?.cancel()
    }

    private func willHide() {
        cover.willHide()
        height = cover.height
        // The frame that ends a hide is what ends the wait. If none comes,
        // the composer is asked to say where it is, since it reports only a
        // change and what it said during the hide was dropped.
        hideTimeout?.cancel()
        hideTimeout = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(1))
            guard !Task.isCancelled, let self, cover.hiding else { return }
            cover.hideTimedOut()
            NotificationCenter.default.post(name: AccessoryHostView.reportAgain, object: scope)
        }
    }

    /// The accessory was resized with no keyboard frame to say so (ov-383).
    ///
    /// With the keys up, UIKit resizes the accessory for a row added above
    /// the field from outside it — a failed send's banner, a queued message,
    /// the plan — and the frame it reported last goes on counting the bar it
    /// had: 456 points for a composer reaching up 516, measured, and the
    /// banner drew over the message it was about. So the accessory says where
    /// its top now is, which is the same overlap a frame would have given.
    /// When a frame does come, it says the same or later, and wins.
    private func apply(accessoryCover top: CGFloat, screenHeight: CGFloat) {
        cover.accessory(cover: top, screenHeight: screenHeight)
        height = cover.height
    }

    private func apply(_ note: Notification) {
        guard
            let frame = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect,
            let screen = (note.object as? UIScreen) ?? UIApplication.shared.connectedScenes
                .compactMap({ ($0 as? UIWindowScene)?.screen }).first
        else { return }
        let overlap = max(0, screen.bounds.height - frame.origin.y)
        // When the accessory's controller takes first responder back after an
        // interactive dismissal, iOS 26 emits a synthetic keyboard frame whose
        // origin is zero. Treating that as geometry reserves the entire screen
        // (932 points on the regression device) and leaves the transcript a
        // tiny strip. `KeyboardCover.frame` reads that as no keyboard; the
        // remaining accessory is measured independently.
        cover.frame(overlap: overlap, screenHeight: screen.bounds.height)
        height = cover.height
    }
}

/// Hosts the bar and keeps it docked.
final class DockedBarController: UIViewController {
    private let host: MeasuredHostingController
    private lazy var bar = AccessoryHostView(host: host)

    init(rootView: AnyView) {
        host = MeasuredHostingController(rootView: rootView)
        super.init(nibName: nil, bundle: nil)
        // The bar re-measuring itself when its OWN content grows.
        //
        // `update(rootView:)` below is the only other `setNeedsLayout` there
        // is, and it runs only when the view that vends the bar is
        // re-evaluated. The draft, the cursor and the attachments are in the
        // pane's `ComposerModel`, which the composer observes — see
        // `AgentComposer` — so typing re-lays this
        // hosting controller out without `AgentView` hearing about it at all.
        // The accessory kept the height one line measured, SwiftUI drew four
        // lines overflowing out of the top of it, and the transcript went on
        // reserving room for a composer eighty points shorter than the one on
        // screen. A message grown to the field's ceiling put the last rows of
        // the conversation back underneath it.
        //
        // A UIKit callback rather than a SwiftUI one: this content is hosted in
        // a SEPARATE SwiftUI graph, so state written from inside it does not
        // invalidate the view that built it. `onHeightChange` already crosses
        // the same boundary the same way, and for the same reason.
        host.onLayout = { [weak self] in self?.bar.setNeedsLayout() }
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

    /// Only the pane on screen may dock. See `DockedBar.isActive`.
    private var isActive = false

    override var canBecomeFirstResponder: Bool { isActive }
    override var inputAccessoryView: UIView? { isActive ? bar : nil }

    func setActive(_ active: Bool) {
        guard active != isActive else { return }
        isActive = active
        if active {
            becomeFirstResponder()
        } else {
            // The controller is not necessarily the first responder — while
            // somebody is typing, the text view INSIDE the accessory is — so
            // resigning here is not enough to put the keyboard away when the
            // pane is switched off screen. Asking the responder chain is.
            KeyboardDismissal.now()
            resignFirstResponder()
        }
        // Without this UIKit keeps showing the accessory it already had; the
        // two properties above are only consulted when inputs are reloaded.
        reloadInputViews()
    }

    /// Whose inset this bar's reports are for.
    var scope: AccessoryScope? {
        get { bar.scope }
        set { bar.scope = newValue }
    }

    /// Called when the bar's measured height changes. Set by the representable.
    var onHeightChange: ((CGFloat) -> Void)? {
        get { bar.onHeightChange }
        set { bar.onHeightChange = newValue }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        // Docked from the start, so the composer is on screen before anybody
        // taps into it — but only if this pane is the one being looked at.
        if isActive { becomeFirstResponder() }
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

/// A hosting controller that says when it has laid its content out.
///
/// `viewDidLayoutSubviews` is the one signal that fires when the SwiftUI inside
/// changes shape on its own — a line added to the draft, a thumbnail strip
/// appearing, the slash-command list opening — none of which reach the view
/// that vends this bar. See `DockedBarController.init`.
final class MeasuredHostingController: UIHostingController<AnyView> {
    /// Called after every layout pass. Re-measuring from here is safe because
    /// `AccessoryHostView` reports only a height that actually CHANGED, so the
    /// pass this provokes settles rather than repeating.
    var onLayout: (() -> Void)?

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        onLayout?()
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
    private let host: MeasuredHostingController

    /// The height last measured, so `layoutSubviews` can tell a real change
    /// from being asked again at the same size.
    private var measured: CGFloat = 0

    /// Reported upward so the conversation can leave room. See `DockedBar`.
    var onHeightChange: ((CGFloat) -> Void)?

    // `AccessoryCoverChannel.didResize` is posted when UIKit has given the
    // accessory a new height, with how far up its window it now reaches
    // (`cover`) and the window's height (`screen`). See
    // `KeyboardInset.apply(accessoryCover:screenHeight:)`.

    /// Asks the accessory to say again where its top is, though its height
    /// hasn't changed: a hide dropped what it said.
    static let reportAgain = Notification.Name("FarCooler.AccessoryHostView.reportAgain")

    /// The bounds height last reported in `didResize`.
    private var reportedHeight: CGFloat = 0

    /// Whose inset hears `didResize`. Nil is any inset's.
    var scope: AccessoryScope?

    private var reportAgainObserver: NSObjectProtocol?

    /// Say so once UIKit has applied a new height: the frame is the laid-out
    /// one here, so its top is where the composer now starts.
    private func reportCover() {
        guard let window, abs(bounds.height - reportedHeight) > 0.5 else { return }
        reportedHeight = bounds.height
        let top = convert(bounds, to: nil).minY
        AccessoryCoverChannel.post(cover: window.bounds.height - top, screen: window.bounds.height, scope: scope)
    }

    init(host: MeasuredHostingController) {
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
        reportAgainObserver = NotificationCenter.default.addObserver(
            forName: Self.reportAgain, object: nil, queue: .main
        ) { [weak self] note in
            MainActor.assumeIsolated {
                // An inset naming no composer asks them all.
                guard let self, note.object == nil || note.object as AnyObject? === self.scope else { return }
                self.reportedHeight = 0
                self.setNeedsLayout()
            }
        }
    }

    deinit {
        reportAgainObserver.map(NotificationCenter.default.removeObserver)
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
        reportCover()
        let height = fittedHeight()
        guard abs(height - measured) > 0.5 else { return }
        measured = height
        invalidateIntrinsicContentSize()
        onHeightChange?(height)
    }
}
