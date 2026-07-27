import AppKit
import SwiftTerm
import SwiftUI

/// The terminal surface.
///
/// SwiftTerm owns VT parsing, the screen grid, the cursor, selection, scrollback
/// and mouse reporting. Overnight owns the transport: bytes in from a live
/// stream, bytes out to the pane. That split is why a full-screen agent behaves
/// correctly here rather than approximately.
@MainActor
final class OvernightTerminalView: TerminalView, @preconcurrency TerminalViewDelegate {
    /// Exact input bytes leaving the terminal, already VT encoded by SwiftTerm.
    var onInput: (([UInt8]) -> Void)?
    /// The emulator's own idea of its grid, which is what the pane must match.
    var onResize: ((Int, Int) -> Void)?

    private var lastSize: (Int, Int) = (0, 0)

    override init(frame: CGRect) {
        super.init(frame: frame)
        terminalDelegate = self
        configure()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        terminalDelegate = self
        configure()
    }

    private func configure() {
        // A terminal is a dark grid of monospaced cells. Match the app's ground.
        nativeBackgroundColor = NSColor(srgbRed: 0.07, green: 0.08, blue: 0.10, alpha: 1)
        nativeForegroundColor = NSColor(srgbRed: 0.86, green: 0.88, blue: 0.91, alpha: 1)
        font = NSFont.monospacedSystemFont(ofSize: 12.5, weight: .regular)
        // Mouse reporting is on by default in SwiftTerm and gated by the program:
        // an agent that asks for mouse events gets them, one that does not still
        // gets ordinary selection.
        optionAsMetaKey = true
    }

    /// Feed bytes from the daemon into the emulator.
    func receive(_ bytes: [UInt8]) {
        feed(byteArray: bytes[...])
    }

    // MARK: - Focus
    //
    // SwiftUI's split view parks first responder in the sidebar, so without
    // claiming it the terminal renders perfectly and silently ignores every
    // keystroke. Claim it when the surface appears and whenever it is clicked.

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        claimKeyboard()
    }

    override func mouseDown(with event: NSEvent) {
        claimKeyboard()
        super.mouseDown(with: event)
    }

    /// Take the keyboard on the next runloop turn, after SwiftUI has finished
    /// installing its own focus.
    func claimKeyboard() {
        DispatchQueue.main.async { [weak self] in
            guard let self, let window = self.window else { return }
            if window.firstResponder !== self {
                window.makeFirstResponder(self)
            }
        }
    }

    // MARK: - TerminalViewDelegate

    func send(source: TerminalView, data: ArraySlice<UInt8>) {
        // SwiftTerm has already encoded the keystroke, chord, arrow, function
        // key or mouse report into exact bytes. Pass them through untouched.
        onInput?(Array(data))
    }

    func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
        guard (newCols, newRows) != lastSize, newCols > 1, newRows > 1 else { return }
        lastSize = (newCols, newRows)
        onResize?(newCols, newRows)
    }

    func setTerminalTitle(source: TerminalView, title: String) {}

    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}

    func scrolled(source: TerminalView, position: Double) {}

    func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}

    func clipboardCopy(source: TerminalView, content: Data) {
        guard let s = String(data: content, encoding: .utf8) else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(s, forType: .string)
    }

    func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
        guard let url = URL(string: link) else { return }
        NSWorkspace.shared.open(url)
    }

    func bell(source: TerminalView) {
        NSSound.beep()
    }

    func iTermContent(source: TerminalView, content: ArraySlice<UInt8>) {}
}

/// SwiftUI wrapper. Owns the stream lifecycle for the selected terminal.
struct TerminalSurface: NSViewRepresentable {
    let terminal: String
    let binary: String?
    let environment: [String: String]
    let onInput: ([UInt8]) -> Void
    /// Resize the pane. Must complete before history is captured.
    let onResize: (Int, Int) async -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    @MainActor
    final class Coordinator {
        var stream: TerminalStream?
        var attached: String?
        var started = false

        func stop() {
            stream?.stop()
            stream = nil
            started = false
        }
    }

    func makeNSView(context: Context) -> OvernightTerminalView {
        let v = OvernightTerminalView(frame: .zero)
        attach(v, context: context)
        return v
    }

    func updateNSView(_ v: OvernightTerminalView, context: Context) {
        // Only re-attach when the selected terminal actually changes. Restarting
        // on every SwiftUI update would reset the screen constantly.
        if context.coordinator.attached != terminal {
            attach(v, context: context)
        }
    }

    /// Wire callbacks and wait for the emulator to report its grid.
    ///
    /// The stream deliberately does NOT start here. Its first act is to replay
    /// the pane's history, and tmux wraps that history at the PANE width. If the
    /// pane is 190 columns while the view is 95, every replayed line wraps a
    /// second time and the screen arrives staggered. So: learn our size, resize
    /// the pane to match, and only then start streaming.
    private func attach(_ v: OvernightTerminalView, context: Context) {
        let coord = context.coordinator
        coord.stop()
        coord.attached = terminal
        v.getTerminal().resetToInitialState()

        v.onInput = onInput
        v.claimKeyboard()

        let box = MainActorBox(v)
        let terminal = self.terminal
        let binary = self.binary
        let environment = self.environment
        let onResize = self.onResize

        /// Resize the pane, then start streaming once.
        ///
        /// Idempotent: whichever path gets here first wins.
        @MainActor
        func begin(_ cols: Int, _ rows: Int) async {
            await onResize(cols, rows)

            guard !coord.started, let binary else { return }
            coord.started = true

            let stream = TerminalStream(onBytes: { bytes in
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { box.value.receive(bytes) }
                }
            })
            stream.start(binary: binary, terminal: terminal, environment: environment)
            coord.stream = stream
        }

        v.onResize = { cols, rows in
            Task { @MainActor in await begin(cols, rows) }
        }

        // Do not wait for a size CHANGE to start.
        //
        // SwiftTerm sizes itself during construction, so that notification has
        // usually already fired by the time this callback is installed, and a
        // window that is never resized would never stream at all. Kick off from
        // the size the emulator already has.
        Task { @MainActor in
            // One turn, so layout has settled and the grid is real.
            try? await Task.sleep(for: .milliseconds(120))
            let t = v.getTerminal()
            let cols = t.cols
            let rows = t.rows
            guard cols > 1, rows > 1 else { return }
            await begin(cols, rows)
        }
    }

    static func dismantleNSView(_ v: OvernightTerminalView, coordinator: Coordinator) {
        coordinator.stop()
    }
}

/// Carries a main-actor value across a callback boundary.
///
/// The stream reader thread hands bytes back, and the view it feeds is
/// main-actor isolated. Every use hops to the main actor before touching the
/// value, so the unchecked conformance is the assertion, not a shortcut.
struct MainActorBox<T>: @unchecked Sendable {
    let value: T
    init(_ value: T) { self.value = value }
}
