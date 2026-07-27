import AppKit
import SwiftUI

/// The terminal surface: an NSView that takes the keyboard directly.
///
/// There is no input box. Keystrokes go to the terminal, which is what makes a
/// full-screen agent usable: Ctrl-C interrupts, arrows navigate its menus, Esc
/// dismisses, and Tab completes, none of which survive a text field that steals
/// Return and swallows control chords.
final class TerminalNSView: NSView {
    var onBytes: (([UInt8]) -> Void)?
    var onGeometry: ((Int, Int) -> Void)?

    private let scroll = NSScrollView()
    private let text = NSTextView()
    private var lastGeometry: (Int, Int) = (0, 0)

    /// Monospaced, because column alignment is the whole point.
    private let font = NSFont.monospacedSystemFont(ofSize: 12.5, weight: .regular)

    override init(frame: NSRect) {
        super.init(frame: frame)
        setUp()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setUp()
    }

    private func setUp() {
        wantsLayer = true
        layer?.backgroundColor = NSColor(srgbRed: 0.07, green: 0.08, blue: 0.10, alpha: 1).cgColor

        text.isEditable = false          // input arrives as key events, not edits
        text.isSelectable = true         // but text stays selectable and copyable
        text.drawsBackground = false
        text.textContainerInset = NSSize(width: 10, height: 8)
        text.isVerticallyResizable = true
        text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]
        text.textContainer?.widthTracksTextView = true

        // tmux has already wrapped the screen to the pane width. Letting the
        // view wrap again would fold lines a second time at a different column
        // and misalign every full-screen TUI.
        text.textContainer?.lineFragmentPadding = 0
        text.isAutomaticQuoteSubstitutionEnabled = false
        text.isAutomaticDashSubstitutionEnabled = false
        text.isAutomaticTextReplacementEnabled = false

        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.autoresizingMask = [.width, .height]
        scroll.frame = bounds
        addSubview(scroll)
    }

    // MARK: - Focus
    //
    // Without these the view never becomes first responder and every keystroke
    // goes somewhere else, which is the bug that made the first build unusable.

    override var acceptsFirstResponder: Bool { true }
    override func becomeFirstResponder() -> Bool { needsDisplay = true; return true }
    override func resignFirstResponder() -> Bool { needsDisplay = true; return true }
    override func mouseDown(with event: NSEvent) { window?.makeFirstResponder(self) }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // Claim the keyboard as soon as the surface appears.
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.window?.makeFirstResponder(self)
        }
    }

    // MARK: - Keyboard

    override func keyDown(with event: NSEvent) {
        // Cmd chords belong to the app (copy, paste, quit), not the terminal.
        if event.modifierFlags.contains(.command) {
            super.keyDown(with: event)
            return
        }
        if let bytes = KeyEncoder.bytes(for: event) {
            onBytes?(bytes)
        }
    }

    /// Cmd-V pastes into the terminal as exact bytes.
    @objc func paste(_ sender: Any?) {
        guard let s = NSPasteboard.general.string(forType: .string) else { return }
        onBytes?(KeyEncoder.bytes(forText: s))
    }

    @objc func copy(_ sender: Any?) {
        guard let range = text.selectedRanges.first as? NSRange, range.length > 0 else { return }
        let selected = (text.string as NSString).substring(with: range)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(selected, forType: .string)
    }

    // MARK: - Content

    func render(_ raw: String) {
        let attributed = ANSIRenderer.attributed(
            raw, font: font,
            defaultFG: NSColor(srgbRed: 0.86, green: 0.88, blue: 0.91, alpha: 1))

        // Preserve the scroll position unless the user is already at the bottom,
        // so reading back through output is not yanked away by a refresh.
        let atBottom = isScrolledToBottom()
        let selected = text.selectedRanges

        text.textStorage?.setAttributedString(attributed)

        if atBottom {
            text.scrollToEndOfDocument(nil)
        } else {
            text.selectedRanges = selected
        }
        reportGeometry()
    }

    private func isScrolledToBottom() -> Bool {
        let visible = scroll.contentView.documentVisibleRect
        let height = text.frame.height
        return visible.maxY >= height - 24
    }

    /// Tell the daemon how many columns and rows we are actually showing, so the
    /// pane is sized to the viewer rather than to a guess.
    ///
    /// Measured from the SCROLL VIEW's content size, not the text container. The
    /// container tracks the text view and is not meaningful until layout has
    /// run, and reading it early yielded a near-zero width that clamped to the
    /// 20 column floor and shrank the real tmux pane to 20x5.
    private func reportGeometry() {
        let charW = ("W" as NSString)
            .size(withAttributes: [.font: font]).width
        let lineH = NSLayoutManager().defaultLineHeight(for: font)
        guard charW > 1, lineH > 1 else { return }

        let usableW = scroll.contentView.bounds.width - text.textContainerInset.width * 2
        let usableH = scroll.contentView.bounds.height - text.textContainerInset.height * 2

        // Before the first real layout the view has no useful size. Reporting
        // then would resize the pane to the clamp floor, so say nothing.
        guard usableW > 120, usableH > 40 else { return }

        let cols = max(20, Int(usableW / charW))
        let rows = max(5, Int(usableH / lineH))

        if (cols, rows) != lastGeometry {
            lastGeometry = (cols, rows)
            onGeometry?(cols, rows)
        }
    }

    override func layout() {
        super.layout()
        scroll.frame = bounds
        reportGeometry()
    }
}

/// SwiftUI wrapper.
struct TerminalSurface: NSViewRepresentable {
    let screen: String
    let onBytes: ([UInt8]) -> Void
    let onGeometry: (Int, Int) -> Void

    func makeNSView(context: Context) -> TerminalNSView {
        let v = TerminalNSView()
        v.onBytes = onBytes
        v.onGeometry = onGeometry
        v.render(screen)
        return v
    }

    func updateNSView(_ v: TerminalNSView, context: Context) {
        v.onBytes = onBytes
        v.onGeometry = onGeometry
        v.render(screen)
    }
}
