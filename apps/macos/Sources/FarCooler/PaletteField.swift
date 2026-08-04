import AppKit
import SwiftUI

/// Which way the highlight should move.
enum PaletteMove {
    case up, down, left, right, next, previous
}

/// The palette's one field.
///
/// An `NSTextView` for the same reason `Composer` is one: SwiftUI's `TextField`
/// keeps the arrow keys for its own caret, and a panel whose ↑ and ↓ move
/// through results while a text field has focus cannot be built out of one
/// without fighting it. Every key that means "move" or "choose" is taken here
/// and everything else falls through to normal editing.
///
/// It also puts the keyboard back where it found it. The terminal claims first
/// responder only when its focus CHANGES — see `TerminalSurface` — so nothing
/// reclaims it after a panel has borrowed it, and a window whose terminal has
/// silently stopped accepting keystrokes is close to impossible to attribute to
/// a panel that is no longer on screen.
struct PaletteField: NSViewRepresentable {
    @Binding var text: String
    var placeholder: String = ""
    /// Whether ← and → move the highlight instead of the caret.
    ///
    /// True in the grid, where there is no text to move through and the second
    /// axis is the whole point of a grid; false in the list, where the caret is
    /// the only thing they could reasonably mean.
    var horizontalMoves: Bool
    let onMove: (PaletteMove) -> Void
    let onSubmit: () -> Void
    let onCancel: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> PaletteTextView {
        let view = PaletteTextView()
        view.delegate = context.coordinator
        view.isRichText = false
        view.drawsBackground = false
        view.font = .systemFont(ofSize: 14)
        view.textContainerInset = NSSize(width: 0, height: 2)
        view.isAutomaticQuoteSubstitutionEnabled = false
        view.isAutomaticDashSubstitutionEnabled = false
        // A query is as often a branch name or a flag as it is prose, and a
        // smart quote in either finds nothing.
        view.isAutomaticTextReplacementEnabled = false
        view.isAutomaticSpellingCorrectionEnabled = false
        view.placeholder = placeholder
        view.string = text
        apply(view)

        context.coordinator.previous = view.window?.firstResponder
        DispatchQueue.main.async {
            context.coordinator.previous = view.window?.firstResponder
            view.window?.makeFirstResponder(view)
        }
        return view
    }

    func updateNSView(_ view: PaletteTextView, context: Context) {
        if view.string != text { view.string = text }
        view.placeholder = placeholder
        apply(view)
    }

    private func apply(_ view: PaletteTextView) {
        view.horizontalMoves = horizontalMoves
        view.onMove = onMove
        view.onSubmit = onSubmit
        view.onCancel = onCancel
    }

    static func dismantleNSView(_ view: PaletteTextView, coordinator: Coordinator) {
        guard let window = view.window, let previous = coordinator.previous else { return }
        // Only if nothing else has taken it. Choosing a terminal from the
        // palette changes the selection, which re-attaches a pane and lets that
        // pane claim the keyboard for itself — restoring over the top of that
        // would type into whatever was on screen a moment ago.
        DispatchQueue.main.async {
            guard window.firstResponder === window || window.firstResponder === view else {
                return
            }
            guard let responder = previous as? NSView, responder.window === window else { return }
            window.makeFirstResponder(responder)
        }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        let parent: PaletteField
        /// Whoever had the keyboard before the panel opened.
        weak var previous: NSResponder?

        init(_ parent: PaletteField) { self.parent = parent }

        func textDidChange(_ notification: Notification) {
            guard let view = notification.object as? NSTextView else { return }
            parent.text = view.string
        }
    }
}

/// An `NSTextView` that gives the navigation keys away.
final class PaletteTextView: NSTextView {
    var horizontalMoves = false
    var onMove: ((PaletteMove) -> Void)?
    var onSubmit: (() -> Void)?
    var onCancel: (() -> Void)?
    var placeholder: String = "" { didSet { needsDisplay = true } }

    override func keyDown(with event: NSEvent) {
        switch Int(event.keyCode) {
        case 36, 76:  // Return, keypad Enter
            onSubmit?()
        case 53:  // Esc
            onCancel?()
        case 48:  // Tab — and Shift-Tab, which every switcher runs backwards
            onMove?(event.modifierFlags.contains(.shift) ? .previous : .next)
        case 126: onMove?(.up)
        case 125: onMove?(.down)
        case 123 where horizontalMoves: onMove?(.left)
        case 124 where horizontalMoves: onMove?(.right)
        default: super.keyDown(with: event)
        }
    }

    /// Return never inserts a line here, whatever route it arrives by.
    override func insertNewline(_ sender: Any?) {
        onSubmit?()
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, !placeholder.isEmpty else { return }

        // Drawn inside the view's own appearance, and in `secondaryLabelColor`.
        //
        // It was `tertiaryLabelColor`, which is about a quarter opaque — enough
        // against a solid background and not against the translucent material
        // this panel floats on, where whatever is behind the window shows through
        // and the hint disappears into it.
        //
        // The appearance matters as much as the color: these are dynamic colors
        // resolved against whatever appearance is current when they are drawn, and
        // an `NSView` hosted inside a SwiftUI overlay is not reliably drawn with
        // its own. Setting it explicitly is what stops a light-mode placeholder
        // being painted in the dark-mode color.
        effectiveAppearance.performAsCurrentDrawingAppearance {
            placeholder.draw(
                at: NSPoint(x: textContainerInset.width + 5, y: textContainerInset.height),
                withAttributes: [
                    .font: font ?? .systemFont(ofSize: 14),
                    .foregroundColor: NSColor.secondaryLabelColor,
                ])
        }
    }
}
