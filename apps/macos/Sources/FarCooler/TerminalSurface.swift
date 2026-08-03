import AppKit
import SwiftUI

/// SwiftUI wrapper around the terminal. Owns the stream lifecycle.
///
/// The split is: this file decides *when* bytes flow, TerminalRenderView draws
/// them, and the Rust core decides what they mean.
struct TerminalSurface: NSViewRepresentable {
    let terminal: String
    let binary: String?
    let environment: [String: String]
    /// Put in front of every launch, to aim it at another machine. See
    /// `DaemonClient.cliHostArguments`.
    let hostArguments: [String]
    /// Resize the pane. Must complete before the stream replays history.
    let onResize: (Int, Int) async -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    @MainActor
    final class Coordinator {
        var stream: TerminalStream?
        var input: TerminalInput?
        var attached: String?
        var started = false
        var fontRevision = 0
        var focused: Bool?
        var pendingGeometry: Task<Void, Never>?

        func stop() {
            pendingGeometry?.cancel()
            pendingGeometry = nil
            stream?.stop()
            stream = nil
            input?.stop()
            input = nil
            started = false
        }
    }

    /// Bumped by preferences, so a font change reaches a live terminal.
    var fontRevision: Int = 0

    /// Whether this pane should own the keyboard.
    ///
    /// Driven by the layout's focus, so the pane with the ring round it is the
    /// pane that receives keystrokes. There is exactly one true at a time.
    var isFocused: Bool = true

    func makeNSView(context: Context) -> TerminalRenderView {
        let view = TerminalRenderView()
        attach(view, context: context)
        return view
    }

    func updateNSView(_ view: TerminalRenderView, context: Context) {
        // Only re-attach when the selected terminal actually changes. Restarting
        // on every SwiftUI update would wipe the screen constantly.
        if context.coordinator.attached != terminal {
            attach(view, context: context)
        }
        if context.coordinator.fontRevision != fontRevision {
            context.coordinator.fontRevision = fontRevision
            view.applyPreferences()
        }
        // Claimed on becoming focused, never on merely existing. Re-asserted on
        // every update because a pane can gain focus long after it mounted — a
        // ⌃B o, a click on another pane, or `farcooler layout focus` from a script.
        if isFocused, context.coordinator.focused != true {
            context.coordinator.focused = true
            view.claimKeyboard()
        } else if !isFocused {
            context.coordinator.focused = false
        }
    }

    static func dismantleNSView(_ view: TerminalRenderView, coordinator: Coordinator) {
        coordinator.stop()
    }

    /// Point the view at a terminal: open the input channel, size the pane to
    /// the view, then stream.
    ///
    /// Order matters. The stream's first act is to replay the pane's history,
    /// and tmux wraps that history at the PANE width. Replaying a 190-column
    /// pane into a 95-column view would wrap every line a second time and the
    /// screen would arrive staggered. So: learn our size, resize the pane to
    /// match, and only then start streaming.
    private func attach(_ view: TerminalRenderView, context: Context) {
        let coord = context.coordinator
        coord.stop()
        coord.attached = terminal

        let terminal = self.terminal
        let binary = self.binary
        let environment = self.environment
        let hostArguments = self.hostArguments
        let onResize = self.onResize

        let input = TerminalInput()
        if let binary {
            input.start(
                binary: binary, terminal: terminal, environment: environment, host: hostArguments)
        }
        coord.input = input
        view.onInput = { bytes in input.send(bytes) }
        coord.focused = nil

        let box = MainActorBox(view)

        // A resize does NOT restart the stream any more.
        //
        // With a real emulator here, the pane resize is enough on its own: tmux
        // sends the program a SIGWINCH, the program redraws itself, and those
        // bytes come down the same stream we are already reading. The emulator
        // reflows what it already holds. Restarting was only ever needed because
        // the old client painted a captured screen it could not reflow.
        view.onGeometry = { columns, rows in
            coord.pendingGeometry?.cancel()
            coord.pendingGeometry = Task { @MainActor in
                // Debounced: a window drag fires this continuously, and each one
                // is a round trip to the daemon.
                try? await Task.sleep(for: .milliseconds(90))
                guard !Task.isCancelled else { return }
                await onResize(columns, rows)

                guard !coord.started, let binary else { return }
                coord.started = true
                let stream = TerminalStream(onBytes: { bytes in
                    DispatchQueue.main.async {
                        MainActor.assumeIsolated { box.value.feed(bytes) }
                    }
                })
                stream.start(
                    binary: binary, terminal: terminal, environment: environment,
                    host: hostArguments)
                coord.stream = stream
            }
        }
    }
}

/// Carries a main-actor value across a callback boundary.
///
/// The stream's reader thread hands bytes back, and the view it feeds is
/// main-actor isolated. Every use hops to the main actor before touching the
/// value, so the unchecked conformance is the assertion, not a shortcut.
struct MainActorBox<T>: @unchecked Sendable {
    let value: T
    init(_ value: T) { self.value = value }
}
