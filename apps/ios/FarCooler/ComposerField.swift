import UIKit
import UniformTypeIdentifiers

/// The agent composer's text view (`ComposerTextView`), which also answers
/// ⌘↩ when it's asked to: the iPad's column composer sends with it (ov-348
/// review R2-3). The field's own key command, because the text view took ⌘↩
/// as a new line before a button's shortcut saw it.
final class ComposerField: UITextView {
    var onCommandReturn: (() -> Void)?

    /// Asked once, when the field first has a window: whether the reader had
    /// the keyboard in the composer this one replaces. Asked then, and not
    /// when the field is made, since the reader may put the keyboard away in
    /// between (ov-357).
    var shouldTakeFocus: (() -> Bool)?

    /// Told when the field leaves its window or is dismantled, so the model
    /// can tell a hand-off to another composer from a pane that's hidden.
    var onTakeDown: (() -> Void)?

    /// Set as the field is leaving its window or being dismantled, so the
    /// resign that comes with it isn't read as the reader putting the
    /// keyboard away. See `ComposerTextView.Coordinator`.
    private(set) var isBeingTakenDown = false

    /// Called by both `willMove(toWindow: nil)` and the dismantle, each of
    /// which the crossing needs (measured); the model counts them as one.
    func takeDown() {
        isBeingTakenDown = true
        onTakeDown?()
    }

    /// Scrolled, the lines beyond the field's edges fade at them instead of
    /// being cut there, as Messages' box does: the text's own inset scrolls
    /// away with its first line (ov-614 review 1). Which edges fade is
    /// `ComposerFade`'s rule: only with text out of view past them, never over
    /// the caret's line (ov-789). A layer mask, which a hosted view takes
    /// where a SwiftUI mask over it didn't draw.
    private let edgeFade = CAGradientLayer()

    /// Told the edges that fade, and whether their fade lies over the caret's
    /// line (`ComposerFade.covers`), whenever either changes: the native
    /// composer's DEBUG probe, for the UI tests.
    var onFade: ((ComposerFade, Bool) -> Void)?
    private var fade = ComposerFade.none
    private var fadeCoversCaret = false

    /// The caret, while there's one in a field being typed in.
    private var caret: CGRect? {
        guard isFirstResponder, let range = selectedTextRange, range.isEmpty else { return nil }
        return caretRect(for: range.end)
    }

    /// The caret moved: its line may have come into an edge's fade, or left it.
    override var selectedTextRange: UITextRange? {
        didSet { setNeedsLayout() }
    }

    /// The width the text was last laid out at, and whether it was scrolled
    /// to its end then.
    private var laidOutWidth: CGFloat = 0
    private var atEnd = true
    /// The width changed while the text was at its end, and nothing has
    /// moved it since: the end is held in view while the text wraps again,
    /// which can finish a layout or two after the width changed. Not while a
    /// caret elsewhere decides what's in view: holding the end took ⌘↑ back
    /// down to it (GlassUITests).
    private var holdsEnd = false

    /// Whether the last line is in view, to the point.
    private var showsEnd: Bool { contentOffset.y + bounds.height >= contentSize.height - 1 }

    /// No caret, or one at the end of the text.
    private var caretAtEnd: Bool {
        !isFirstResponder || selectedRange.location >= (text as NSString).length
    }

    private func keepEnd() {
        guard holdsEnd else { return }
        if isTracking || !caretAtEnd {
            holdsEnd = false
        } else if !showsEnd {
            setContentOffset(CGPoint(x: contentOffset.x, y: max(contentSize.height - bounds.height, 0)), animated: false)
        }
    }

    override var contentSize: CGSize {
        didSet { keepEnd() }
    }

    override func becomeFirstResponder() -> Bool {
        // The caret decides what's in view from here.
        holdsEnd = false
        return super.becomeFirstResponder()
    }

    override func layoutSubviews() {
        // A field scrolled to its end stays there when its width changes: the
        // bar narrows as the keyboard goes down (ov-804), the draft wraps
        // onto more lines, and the end of it went out of view below.
        if bounds.width != laidOutWidth, laidOutWidth > 0, atEnd, caretAtEnd { holdsEnd = true }
        super.layoutSubviews()
        keepEnd()
        laidOutWidth = bounds.width
        atEnd = showsEnd
        let caret = self.caret
        let now = ComposerFade.edges(
            offset: contentOffset.y, height: bounds.height, content: contentSize.height,
            insetTop: textContainerInset.top, insetBottom: textContainerInset.bottom,
            caretTop: caret?.minY, caretBottom: caret?.maxY)
        let covers = caret.map {
            now.covers(caretTop: $0.minY, caretBottom: $0.maxY, offset: contentOffset.y, height: bounds.height)
        } ?? false
        if now != fade || covers != fadeCoversCaret {
            fade = now
            fadeCoversCaret = covers
            onFade?(now, covers)
        }
        let above = now.above, below = now.below
        guard above || below else {
            if layer.mask != nil { layer.mask = nil }
            return
        }
        let edge = NSNumber(value: Double(ComposerFade.depth / bounds.height))
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        edgeFade.frame = bounds
        edgeFade.colors = [
            (above ? UIColor.clear : UIColor.black).cgColor, UIColor.black.cgColor,
            UIColor.black.cgColor, (below ? UIColor.clear : UIColor.black).cgColor,
        ]
        edgeFade.locations = [0, edge, NSNumber(value: 1 - edge.doubleValue), 1]
        if layer.mask !== edgeFade { layer.mask = edgeFade }
        CATransaction.commit()
    }

    override func willMove(toWindow newWindow: UIWindow?) {
        if newWindow == nil { takeDown() }
        super.willMove(toWindow: newWindow)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil else { return }
        isBeingTakenDown = false
        if let ask = shouldTakeFocus {
            shouldTakeFocus = nil
            if ask() { becomeFirstResponder() }
        }
    }

    /// Told when a paste is images, with each one's bytes (ov-404): the
    /// conversation composer takes them as chips. Nil leaves a paste to the
    /// text view, which is the agent composer's.
    var onPasteImages: (([Data]) -> Void)?

    /// The bytes of the images on `pasteboard`, or none when it holds words
    /// to paste instead: any text but a lone web address, which a browser's
    /// Copy Image may put beside the image it copied (as the Mac does).
    static func images(on pasteboard: UIPasteboard) -> [Data] {
        if pasteboard.hasStrings {
            let words = pasteboard.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let lone = !words.contains(where: \.isWhitespace) && ["http", "https"].contains(URL(string: words)?.scheme ?? "")
            if !(lone && pasteboard.hasImages) { return [] }
        }
        guard pasteboard.hasImages else { return [] }
        // The bytes as they were copied: a screenshot's PNG, a photo's JPEG
        // or HEIC, rather than a UIImage encoded again.
        let types: [UTType] = [.png, .jpeg, .heic, .gif, .webP, .tiff]
        var found: [Data] = []
        for item in pasteboard.items {
            if let data = types.lazy.compactMap({ item[$0.identifier] as? Data }).first { found.append(data) }
        }
        if found.isEmpty, let data = pasteboard.image?.pngData() { found = [data] }
        return found
    }

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        // `hasImages` only: it never reads the contents, so it can't raise the
        // "Allow Paste" prompt each time the edit menu asks. What is pasted is
        // read in `paste`.
        if action == #selector(paste(_:)), onPasteImages != nil, UIPasteboard.general.hasImages { return true }
        return super.canPerformAction(action, withSender: sender)
    }

    override func paste(_ sender: Any?) {
        if let onPasteImages {
            let images = Self.images(on: .general)
            if !images.isEmpty {
                onPasteImages(images)
                return
            }
        }
        super.paste(sender)
    }

    /// Whether Tab from a hardware keyboard takes something now (the native
    /// composer's suggestion, ov-409), asked as each key is pressed; when it
    /// doesn't, Tab is a Tab.
    var offersTab: (() -> Bool)?
    var onTab: (() -> Void)?

    override var keyCommands: [UIKeyCommand]? {
        var commands = super.keyCommands ?? []
        if onCommandReturn != nil {
            let send = UIKeyCommand(title: "Send", action: #selector(commandReturn), input: "\r", modifierFlags: .command)
            send.wantsPriorityOverSystemBehavior = true
            commands.append(send)
        }
        if offersTab?() == true {
            let take = UIKeyCommand(title: "Use Suggestion", action: #selector(takeTab), input: "\t", modifierFlags: [])
            take.wantsPriorityOverSystemBehavior = true
            commands.append(take)
        }
        return commands.isEmpty ? super.keyCommands : commands
    }

    @objc private func takeTab() { onTab?() }

    @objc private func commandReturn() { onCommandReturn?() }
}
