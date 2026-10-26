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
    /// away with its first line (ov-614 review 1). An edge fades only while
    /// TEXT is out of view past it, and never over the caret's line (ov-789).
    /// A layer mask, which a hosted view takes where a SwiftUI mask over it
    /// didn't draw.
    ///
    /// The bottom used to fade "while there's more below", and the text's
    /// own 2 pt inset counted as more: scrolled to the caret at the end of a
    /// draft taller than the field, the text view stops with the caret's line
    /// on the edge and the inset still under it, so the fade sat on that line
    /// and cut the tails of its p's and g's, on every line typed past the
    /// field's height.
    private let edgeFade = CAGradientLayer()
    static let fadeHeight: CGFloat = 8

    /// Told the edges that fade whenever they change: the native composer's
    /// DEBUG probe, for the UI tests.
    var onFade: ((Fade) -> Void)?
    private var fade = Fade.none

    /// Which edges fade, and whether the caret's line is under one of them.
    struct Fade: Equatable {
        var above = false
        var below = false
        var caretFaded = false
        static let none = Fade()
    }

    /// The edges to fade, for a text view scrolled to `offset`, `height`
    /// tall, over `content` of text and its `insets`: an edge with text out
    /// of view past it, unless the caret's line is inside that edge's fade.
    static func fade(offset: CGFloat, height: CGFloat, content: CGFloat, insets: UIEdgeInsets, caret: CGRect?) -> Fade {
        guard height > fadeHeight * 2 else { return .none }
        let top = offset, bottom = offset + height
        var above = top > insets.top + 0.5
        var below = bottom < content - insets.bottom - 0.5
        if let caret {
            if caret.maxY > bottom - fadeHeight { below = false }
            if caret.minY < top + fadeHeight { above = false }
        }
        // Read off the result, not the rule: whatever decided the edges, is
        // the caret's line under one?
        let faded = caret.map { (above && $0.minY < top + fadeHeight) || (below && $0.maxY > bottom - fadeHeight) } ?? false
        return Fade(above: above, below: below, caretFaded: faded)
    }

    /// The caret, while there's one in a field being typed in.
    private var caret: CGRect? {
        guard isFirstResponder, let range = selectedTextRange, range.isEmpty else { return nil }
        return caretRect(for: range.end)
    }

    /// The caret moved: its line may have come into an edge's fade, or left it.
    override var selectedTextRange: UITextRange? {
        didSet { setNeedsLayout() }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let now = Self.fade(
            offset: contentOffset.y, height: bounds.height, content: contentSize.height, insets: textContainerInset,
            caret: caret)
        if now != fade {
            fade = now
            onFade?(now)
        }
        let above = now.above, below = now.below
        guard above || below else {
            if layer.mask != nil { layer.mask = nil }
            return
        }
        let edge = NSNumber(value: Double(Self.fadeHeight / bounds.height))
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
