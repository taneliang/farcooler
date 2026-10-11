import AgentKit
import AppKit
import SwiftUI

/// The native composer's text box (ov-400): the chat composer's own
/// `ComposerTextView`, because a SwiftUI field can't tell Return from
/// Shift-Return, break a line at the caret, or take a pasted or dropped image.
///
/// - Return sends; Shift-Return breaks the line, which a runner without
///   `compose` gets as a space (`NativePaneModel.draft`). While an input
///   method composes, Return is the input method's; the tiling prefix sees
///   every key first.
/// - Paste and drop take images where the runner takes them
///   (`NativePaneModel.attach`), else text as usual.
/// - It grows from one line to eight with what's in it, then scrolls. Its
///   height is reported after the text changes, never measured during
///   layout (`AgentComposerField.measuredHeight` says why).
struct ComposerField: NSViewRepresentable {
    @ObservedObject var model: NativePaneModel
    let isFocused: Bool
    @Binding var measuredHeight: CGFloat

    /// The most lines shown before the box scrolls.
    static let mostLines: CGFloat = 8

    static var font: NSFont { .preferredFont(forTextStyle: .body) }

    /// One line's height, the box's least.
    static var lineHeight: CGFloat { NSLayoutManager().defaultLineHeight(for: font).rounded(.up) }

    func makeCoordinator() -> Coordinator { Coordinator(model: model, height: $measuredHeight) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.borderType = .noBorder
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        let view = ComposerTextView()
        view.delegate = context.coordinator
        view.isRichText = false
        FieldUndo.enable(view)
        view.isVerticallyResizable = true
        view.isHorizontallyResizable = false
        view.autoresizingMask = [.width]
        view.minSize = .zero
        view.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        view.textContainer?.widthTracksTextView = true
        view.textContainer?.heightTracksTextView = false
        view.textContainer?.lineFragmentPadding = 0
        view.textContainerInset = .zero
        view.drawsBackground = false
        view.font = Self.font
        view.isAutomaticQuoteSubstitutionEnabled = false
        view.isAutomaticDashSubstitutionEnabled = false
        view.isAutomaticTextReplacementEnabled = false
        view.registerForDraggedTypes([.png, .tiff])
        view.string = model.draft
        view.setAccessibilityIdentifier("native-composer-text")
        view.setAccessibilityLabel("Message \(model.agent)")
        scroll.documentView = view
        context.coordinator.view = view
        apply(view, context: context)
        context.coordinator.report(view)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let view = scroll.documentView as? ComposerTextView else { return }
        context.coordinator.model = model
        if view.string != model.draft, !view.hasMarkedText() {
            context.coordinator.undo.replace(view, with: model.draft)
            context.coordinator.report(view)
        }
        apply(view, context: context)
        // Claimed on becoming focused, never merely on existing, as the chat
        // composer's field is.
        if isFocused, context.coordinator.focused != true {
            context.coordinator.focused = true
            DispatchQueue.main.async { view.window?.makeFirstResponder(view) }
        } else if !isFocused {
            context.coordinator.focused = false
        }
    }

    private func apply(_ view: ComposerTextView, context: Context) {
        view.takesKeyboard = !context.environment.outOfSight
        let coordinator = context.coordinator
        view.onSubmit = { coordinator.send() }
        view.onTab = { coordinator.takeSuggestion() }
        view.onImages = { coordinator.take(from: $0) }
        view.offersImages = { coordinator.offers($0) }
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var model: NativePaneModel
        let height: Binding<CGFloat>
        weak var view: ComposerTextView?
        var focused: Bool?
        /// The height last sent up, one line to start.
        var reported = ComposerField.lineHeight
        /// This field's own undo. See `FieldUndo`.
        let undo = FieldUndo()

        init(model: NativePaneModel, height: Binding<CGFloat>) {
            self.model = model
            self.height = height
        }

        func undoManager(for view: NSTextView) -> UndoManager? { undo.manager }

        func textDidChange(_ notification: Notification) {
            guard let view = notification.object as? NSTextView else { return }
            if model.draft != view.string { model.draft = view.string }
            report(view)
        }

        /// Tab with a suggestion on offer (ov-409): its words typed into the
        /// empty box, so the caret ends after them and Undo takes them out.
        /// Nothing is sent; false, and Tab is a Tab, when none is on offer.
        func takeSuggestion() -> Bool {
            guard let words = model.suggestion, let view else { return false }
            view.insertText(words, replacementRange: view.selectedRange())
            return true
        }

        func send() {
            let model = model
            Task { await model.send() }
        }

        /// The images on `pasteboard`, and where the runner takes them its
        /// other files (ov-454), added to the message: false when the runner
        /// takes none, or there are none, so the text view pastes or drops
        /// as usual.
        func take(from pasteboard: NSPasteboard) -> Bool {
            guard model.rich else { return false }
            let images = ComposeImage.from(pasteboard)
            let files = model.takesFiles ? ComposeFile.urls(on: pasteboard) : []
            guard !images.isEmpty || !files.isEmpty else { return false }
            if !images.isEmpty { model.attach(images) }
            if !files.isEmpty { model.attach(fileURLs: files) }
            return true
        }

        func offers(_ pasteboard: NSPasteboard) -> Bool {
            model.rich && (ComposeImage.offered(on: pasteboard) || model.takesFiles && !ComposeFile.urls(on: pasteboard).isEmpty)
        }

        /// How tall the text is now, one line to eight, sent up as state.
        func report(_ view: NSTextView) {
            guard let container = view.textContainer, let manager = view.layoutManager else { return }
            let line = ComposerField.lineHeight
            // Laid out until the width holds. Text taller than the box brings
            // in a scroll bar where scroll bars take room (no trackpad or
            // mouse, as on CI's runner), which narrows the text in the middle
            // of laying it out and throws that layout away: three lines set
            // from outside measured as two, and the box stayed short (ov-470).
            for _ in 0..<3 {
                let width = container.size.width
                manager.ensureLayout(for: container)
                if container.size.width == width { break }
            }
            // Empty is one line, stated rather than measured (ov-470).
            let used = view.string.isEmpty ? line : manager.usedRect(for: container).height
            let clamped = min(max(used, line), line * ComposerField.mostLines).rounded(.up)
            // Against the last height this sent, never the binding's own
            // value: the binding is the one made with the coordinator, and
            // reads the first height forever, so a box cleared by Send
            // matched it and never shrank (ov-470).
            guard abs(clamped - reported) > 0.5 else { return }
            reported = clamped
            let height = height
            DispatchQueue.main.async { height.wrappedValue = clamped }
        }
    }
}
