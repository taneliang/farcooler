import AppKit
import SwiftUI

/// A terminal pane: the emulator, and whatever is floating over it.
///
/// A thin wrapper over `TerminalCanvas`, which is the part that has always been
/// here. It exists so an image being pasted has somewhere to be drawn: the
/// canvas is an `NSViewRepresentable` and cannot overlay SwiftUI on itself, and
/// putting the chip at both call sites instead would be the same view written
/// twice.
struct TerminalSurface: View {
    let terminal: String
    let binary: String?
    let environment: [String: String]
    let hostArguments: [String]
    let linkGeneration: Int
    let onResize: (Int, Int) async -> Void
    var fontRevision: Int = 0
    var isFocused: Bool = true

    /// One queue per pane, so two panes uploading at once each show their own.
    @StateObject private var pastes = ImagePasteQueue()

    var body: some View {
        TerminalCanvas(
            terminal: terminal,
            binary: binary,
            environment: environment,
            hostArguments: hostArguments,
            linkGeneration: linkGeneration,
            onResize: onResize,
            pastes: pastes,
            fontRevision: fontRevision,
            isFocused: isFocused
        )
        .overlay(alignment: .bottom) { ImagePasteChips(queue: pastes) }
    }
}

/// SwiftUI wrapper around the terminal. Owns the stream lifecycle.
///
/// The split is: this file decides *when* bytes flow, TerminalRenderView draws
/// them, and the Rust core decides what they mean.
struct TerminalCanvas: NSViewRepresentable {
    let terminal: String
    let binary: String?
    let environment: [String: String]
    /// Put in front of every launch, to aim it at another runner. See
    /// `DaemonClient.cliHostArguments`.
    let hostArguments: [String]
    /// Bumped by `DaemonClient` when this runner's link is replaced, so a
    /// pane whose stream died with the old one opens a new one.
    ///
    /// Without it a remote runner dropping killed the `farcooler terminal
    /// stream` subprocess behind every pane, and nothing ever started another:
    /// `attach` runs when the pane changes terminals, and this pane is still
    /// showing the same terminal it always was. The runner came back, the
    /// sidebar went green, and every pane stayed frozen on its last byte.
    let linkGeneration: Int
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
        /// Which link the live stream was opened on. `nil` until one has been.
        var linkGeneration: Int?

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

    /// Where a pasted image goes. Held rather than passed through a closure so
    /// the canvas can give it the one thing only the view can do: type.
    let pastes: ImagePasteQueue

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
        // Only re-attach when the selected terminal actually changes, or when
        // the link underneath it has been replaced. Restarting on every
        // SwiftUI update would wipe the screen constantly.
        //
        // The generation is compared rather than merely checked against zero
        // so a pane that mounts onto an already-reconnected runner records
        // where it started and re-attaches only on the NEXT drop, instead of
        // re-attaching once for a reconnection that happened before it existed.
        if context.coordinator.attached != terminal
            || context.coordinator.linkGeneration != linkGeneration
        {
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
        coord.linkGeneration = linkGeneration

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

        // Typing is the view's, because only it holds the emulator that knows
        // whether the program asked for bracketing. Everything else about a
        // paste — copying the bytes, naming the file, expiring it — belongs to
        // the daemon, and the queue is what asks it.
        let pastes = self.pastes
        pastes.type = { [weak view] text in view?.typePaste(text) }
        view.onPasteImage = { pasted in
            pastes.start(
                pasted, terminal: terminal, binary: binary,
                environment: environment, hostArguments: hostArguments)
        }

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
            // Whether this is the report that OPENS the stream, decided here
            // rather than inside the task: two reports can arrive in the same
            // layout pass, and both would read `coord.started` as false if they
            // read it after their own first `await`.
            let opening = !coord.started
            coord.pendingGeometry = Task { @MainActor in
                // Debounced, but only once there is something to protect. A
                // window drag fires this continuously and each one is a round
                // trip to the daemon, so a settled size is worth waiting for —
                // whereas the FIRST report is the one the stream is waiting on,
                // and nothing is going to supersede it. Sleeping through it just
                // held the pane blank for another 90ms on every mount, which is
                // every layout switch and every reconnect.
                if !opening {
                    try? await Task.sleep(for: .milliseconds(90))
                    guard !Task.isCancelled else { return }
                }
                await onResize(columns, rows)
                // Re-checked after the resize, not only before it: `onResize` is
                // a round trip, and a newer report can land while it is in
                // flight. Without this the older size would go on to open the
                // stream that the newer one is entitled to.
                guard !Task.isCancelled else { return }

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

        // Ask for the report rather than waiting for one.
        //
        // At mount this does nothing — the view has no bounds yet, so
        // `reportGeometry` returns early and the real report arrives from
        // `layout()`. On a re-attach it is the whole mechanism: the grid is
        // unchanged, so no layout pass is coming, and without a report the
        // closure above never runs and no stream is ever opened.
        view.reannounceGeometry()
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
