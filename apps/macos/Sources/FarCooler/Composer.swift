import AppKit
import SwiftUI

/// A prompt field that behaves the way a prompt field should.
///
/// SwiftUI's multi-line `TextField` makes Return insert a newline and never
/// fires `onSubmit`, which is the opposite of what a message composer needs.
/// Every chat application in existence uses Return to send and Shift-Return for
/// a line break, and a prompt to an agent is a message.
///
/// So this is an `NSTextView`: Return sends, Shift-Return breaks the line, Esc
/// closes, and the field grows with the text up to a limit.
struct Composer: NSViewRepresentable {
    @Binding var text: String
    var placeholder: String = ""
    var minHeight: CGFloat = 24
    var maxHeight: CGFloat = 220
    let onSubmit: () -> Void
    let onCancel: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = false
        scroll.autohidesScrollers = true

        let view = SubmittingTextView()
        view.delegate = context.coordinator
        view.onSubmit = onSubmit
        view.onCancel = onCancel
        view.isRichText = false
        view.drawsBackground = false
        view.font = .systemFont(ofSize: 14)
        view.textContainerInset = NSSize(width: 0, height: 3)
        view.isAutomaticQuoteSubstitutionEnabled = false
        view.isAutomaticDashSubstitutionEnabled = false
        // A prompt is code as often as prose; "helpful" substitutions corrupt it.
        view.isAutomaticTextReplacementEnabled = false
        view.placeholder = placeholder
        view.string = text

        scroll.documentView = view
        context.coordinator.view = view
        DispatchQueue.main.async { view.window?.makeFirstResponder(view) }
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let view = scroll.documentView as? SubmittingTextView else { return }
        if view.string != text { view.string = text }
        view.onSubmit = onSubmit
        view.onCancel = onCancel
        view.placeholder = placeholder
    }

    /// How tall the text wants to be, clamped.
    func height(for text: String, width: CGFloat) -> CGFloat {
        let font = NSFont.systemFont(ofSize: 14)
        let rect = (text.isEmpty ? " " : text).boundingRect(
            with: NSSize(width: max(width, 1), height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading],
            attributes: [.font: font])
        return min(max(ceil(rect.height) + 6, minHeight), maxHeight)
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        let parent: Composer
        weak var view: SubmittingTextView?

        init(_ parent: Composer) { self.parent = parent }

        func textDidChange(_ notification: Notification) {
            guard let view = notification.object as? NSTextView else { return }
            parent.text = view.string
        }
    }
}

/// An `NSTextView` that sends on Return.
final class SubmittingTextView: NSTextView {
    var onSubmit: (() -> Void)?
    var onCancel: (() -> Void)?
    var placeholder: String = "" { didSet { needsDisplay = true } }

    override func keyDown(with event: NSEvent) {
        let isReturn = event.keyCode == 36 || event.keyCode == 76
        if isReturn && !event.modifierFlags.contains(.shift) {
            onSubmit?()
            return
        }
        if event.keyCode == 53 {  // Esc
            onCancel?()
            return
        }
        super.keyDown(with: event)
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, !placeholder.isEmpty else { return }
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font ?? .systemFont(ofSize: 14),
            .foregroundColor: NSColor.tertiaryLabelColor,
        ]
        placeholder.draw(
            at: NSPoint(x: textContainerInset.width + 5, y: textContainerInset.height),
            withAttributes: attributes)
    }
}

/// The agents Far Cooler knows how to launch, and the models worth offering.
///
/// A short curated list plus "Default", not an exhaustive one. Model names
/// change faster than an app ships, so the agent's own default is always
/// available and anything not listed can be typed in Settings.
enum Agents {
    struct Agent: Identifiable, Hashable {
        let id: String
        let name: String
        let models: [String]
    }

    static let all: [Agent] = [
        Agent(id: "claude", name: "Claude Code", models: ["opus", "sonnet", "haiku"]),
        Agent(id: "codex", name: "Codex", models: ["gpt-5.6-sol", "gpt-5.6-sol-high"]),
        Agent(id: "cursor", name: "Cursor", models: ["auto", "sonnet-4.5", "gpt-5"]),
    ]

    static func agent(_ id: String) -> Agent {
        all.first { $0.id == id } ?? all[0]
    }

    /// The preset string the daemon expects: `agent` or `agent:model`.
    static func preset(agent: String, model: String) -> String {
        model.isEmpty ? agent : "\(agent):\(model)"
    }
}
