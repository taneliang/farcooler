import AgentKit
import AppKit

/// An `NSTextView` that sends on Return, hands its arrows to an open picker,
/// and gives the tiling prefix first refusal on every key it sees.
///
/// That last part is the load-bearing one. `PaletteField`'s own doc comment
/// explains why a plain `NSTextView` cannot be trusted with ⌃B on its own:
/// once this field is first responder, nothing else in the chain ever sees
/// the keystroke before `NSTextView` would insert or ignore it. Without this
/// override, focusing the composer to type a message would silently disable
/// every `⌃B` binding and the prefix-less `⌃HJKL` pane traversal until focus
/// moved elsewhere — exactly the failure `TerminalRenderView.keyDown` already
/// guards against for a terminal, and the conversation view's composer needs
/// the same guard.
final class ComposerTextView: NSTextView {
    /// Off while out of sight (`outOfSight`): see `TerminalRenderView`.
    var takesKeyboard = true {
        didSet { if !takesKeyboard { KeyboardFence.release(self) } }
    }

    override var acceptsFirstResponder: Bool { takesKeyboard && super.acceptsFirstResponder }

    override func becomeFirstResponder() -> Bool {
        takesKeyboard && super.becomeFirstResponder()
    }

    var pickerOpen = false
    var onNavigate: ((PaletteMove) -> Void)?
    var onAccept: (() -> Void)?
    var onSubmit: (() -> Void)?
    var onDismissPicker: (() -> Void)?
    var onPasteImage: ((NSImage) -> Void)?
    var onApprove: (() -> Void)?
    var onReject: (() -> Void)?
    /// Tab with nothing else to do: true when it took the key (the native
    /// composer's suggestion, ov-409), else Tab is a Tab.
    var onTab: (() -> Bool)?
    var placeholder: String = "" { didSet { needsDisplay = true } }
    /// Where Paste reads: the system's pasteboard, or a test's own.
    var pasteboard: NSPasteboard = .general
    /// Takes the images on a pasteboard, pasted or dropped, before anything
    /// else does; true when it took them (the native composer, ov-400).
    var onImages: ((NSPasteboard) -> Bool)?
    /// Whether a pasteboard holds images `onImages` would take, for a drag
    /// passing over.
    var offersImages: ((NSPasteboard) -> Bool)?

    override func keyDown(with event: NSEvent) {
        // An input method is composing (Japanese, Chinese, Korean) and Return
        // commits its candidate: that Return is the input method's, and taking
        // it would send half-composed text (ov-162). Nothing below is for a key
        // the input method owns, so it is first.
        if hasMarkedText() {
            super.keyDown(with: event)
            return
        }
        if MainActor.assumeIsolated({ PrefixMode.shared.handle(event) }) == .handled {
            return
        }

        // Answering the agent, from wherever the cursor happens to be.
        //
        // The composer holds first responder for the whole pane, so a shortcut
        // attached only to the buttons would be dead exactly when a user is
        // most likely to reach for it — mid-sentence, with the agent waiting.
        //
        // ⌘ and nothing else. `.contains(.command)` also let ⇧⌘↩ approve,
        // which nobody was told about and which is Zoom Pane's chord now.
        if event.modifierFlags.intersection([.command, .shift, .option, .control]) == .command {
            switch Int(event.keyCode) {
            case 36, 76:  // Return, keypad Enter
                if let onApprove { onApprove(); return }
            case 51:  // Delete
                if let onReject { onReject(); return }
            default: break
            }
        }

        if pickerOpen {
            switch Int(event.keyCode) {
            case 126: onNavigate?(.up); return
            case 125: onNavigate?(.down); return
            case 48: onNavigate?(event.modifierFlags.contains(.shift) ? .previous : .next); return
            case 36, 76: onAccept?(); return  // Return, keypad Enter
            case 53: onDismissPicker?(); return  // Esc closes the picker, not the compose
            default: break
            }
        }

        if event.keyCode == 48, event.modifierFlags.intersection([.command, .shift, .option, .control]).isEmpty,
            onTab?() == true
        {
            return
        }

        let isReturn = event.keyCode == 36 || event.keyCode == 76
        if isReturn && !event.modifierFlags.contains(.shift) {
            onSubmit?()
            return
        }
        // Shift-Return breaks the line here, from the event in hand, rather
        // than through `insertNewline`, which reads the shift off
        // `NSApp.currentEvent`.
        if isReturn {
            insertNewlineIgnoringFieldEditor(nil)
            return
        }
        super.keyDown(with: event)
    }

    /// Belt and braces: some input methods deliver Return through
    /// `insertNewline` rather than `keyDown`, and `Composer`'s own field
    /// overrides this too for the same reason.
    override func insertNewline(_ sender: Any?) {
        // Shift+Return is a line break, and this is where it was being eaten.
        //
        // `keyDown` above deliberately lets it through to `super` — and
        // `super`'s answer to Return, shift or not, is to call this method. So
        // the fallback below swallowed the exact keystroke `keyDown` had just
        // taken care to let past, and a multi-line message was impossible to
        // type.
        if NSApp.currentEvent?.modifierFlags.contains(.shift) == true {
            super.insertNewline(sender)
            return
        }
        // The same Return by the other route: composing, it is the input
        // method's.
        guard !hasMarkedText() else { return }
        if pickerOpen { onAccept?() } else { onSubmit?() }
    }

    override func paste(_ sender: Any?) {
        if let onImages, onImages(pasteboard) { return }
        if let onPasteImage, let image = NSImage(pasteboard: pasteboard) {
            onPasteImage(image)
            return
        }
        super.paste(sender)
    }

    /// Paste enabled for images `onImages` takes (ov-454). A plain-text
    /// view reads no image type, so on its own it greys Paste out for a
    /// copied screenshot and ⌘V never reaches `paste`.
    override func validateUserInterfaceItem(_ item: any NSValidatedUserInterfaceItem) -> Bool {
        if item.action == #selector(paste(_:)), isEditable {
            if offersImages?(pasteboard) == true { return true }
            // Paste reads `pasteboard`, so it's judged by it: AppKit's own
            // rule reads the clipboard, whatever this pastes from.
            if pasteboard != .general { return pasteboard.availableType(from: readablePasteboardTypes) != nil }
        }
        return super.validateUserInterfaceItem(item)
    }

    /// A drop of image data alone: a plain-text view refuses it here, before
    /// `performDragOperation`, for the same reason as Paste (ov-454).
    override func prepareForDragOperation(_ sender: any NSDraggingInfo) -> Bool {
        offersImages?(sender.draggingPasteboard) == true || super.prepareForDragOperation(sender)
    }

    override func draggingEntered(_ sender: any NSDraggingInfo) -> NSDragOperation {
        offersImages?(sender.draggingPasteboard) == true ? .copy : super.draggingEntered(sender)
    }

    override func draggingUpdated(_ sender: any NSDraggingInfo) -> NSDragOperation {
        offersImages?(sender.draggingPasteboard) == true ? .copy : super.draggingUpdated(sender)
    }

    override func performDragOperation(_ sender: any NSDraggingInfo) -> Bool {
        if let onImages, onImages(sender.draggingPasteboard) { return true }
        return super.performDragOperation(sender)
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, !placeholder.isEmpty else { return }
        effectiveAppearance.performAsCurrentDrawingAppearance {
            placeholder.draw(
                at: NSPoint(x: textContainerInset.width + 5, y: textContainerInset.height),
                withAttributes: [
                    .font: font ?? .systemFont(ofSize: 13),
                    .foregroundColor: NSColor.tertiaryLabelColor,
                ])
        }
    }
}
