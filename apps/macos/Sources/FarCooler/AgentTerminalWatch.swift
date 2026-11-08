import AppKit
import SwiftUI

/// A running engine agent's terminal, to look at and nothing more (ov-972).
///
/// The bytes come from `farcooler workflow watch`, which only ever prints. This view hands the emulator
/// no way back: no `onInput` (so neither a key, a paste nor the emulator's own replies to a program's
/// questions go anywhere), no geometry report (so the agent's pane is never resized), no keyboard.
/// What is left is the picture, scrolling, and selecting text to copy.
struct AgentTerminalWatch: View {
    let workspace: String
    let item: String
    let binary: String?
    let environment: [String: String]
    let hostArguments: [String]
    let linkGeneration: Int

    @Environment(\.dismiss) private var dismiss
    @State private var gotBytes = false
    /// Nil while the stream runs. The exit code is the one thing that ends the watch, so no frame can
    /// say "Finished" before the code that says otherwise has arrived.
    @State private var exitStatus: Int32?

    /// What to tell the owner when the stream stops: nothing while it runs, "finished" only when the
    /// watch ended cleanly (the engine reaped the pane), a dropped link when it did not, and with no
    /// picture either the reason (a review, a merge, or an agent that already ended) or, after a
    /// failed exit, that the agent could not be reached.
    static func status(gotBytes: Bool, exitStatus: Int32?) -> String? {
        guard let exitStatus else { return nil }
        guard gotBytes else { return exitStatus == 0 ? "This agent isn’t running right now." : "Couldn’t reach this agent." }
        return exitStatus == 0 ? "Finished" : "Lost the connection to this agent."
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Watching \(item)").font(.headline)
                Text("Read-only").font(.caption).foregroundStyle(.secondary)
                if gotBytes, let done = Self.status(gotBytes: gotBytes, exitStatus: exitStatus) {
                    Text(done).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Close") { dismiss() }.keyboardShortcut(.cancelAction)
            }
            .padding(10)
            Divider()
            if binary == nil {
                Text("This runner isn’t reachable right now.").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                AgentTerminalCanvas(
                    item: item, command: AgentTerminalCanvas.command(workspace: workspace, item: item),
                    binary: binary, environment: environment, hostArguments: hostArguments,
                    linkGeneration: linkGeneration,
                    onBytes: { gotBytes = true }, onExit: { exitStatus = $0 })
                    .overlay {
                        if let why = Self.status(gotBytes: gotBytes, exitStatus: exitStatus), !gotBytes {
                            Text(why).foregroundStyle(.secondary)
                        }
                    }
                    .id("\(workspace)/\(item)#\(linkGeneration)")
            }
        }
        .frame(minWidth: 640, minHeight: 420)
    }
}

struct AgentTerminalCanvas: NSViewRepresentable {
    let item: String
    let command: [String]
    let binary: String?
    let environment: [String: String]
    let hostArguments: [String]
    let linkGeneration: Int
    var onBytes: @MainActor @Sendable () -> Void = {}
    var onEnd: @MainActor @Sendable () -> Void = {}
    var onExit: @MainActor @Sendable (Int32) -> Void = { _ in }

    /// The whole of what this view ever asks the CLI to do. Pinned by a test.
    static func command(workspace: String, item: String) -> [String] { ["workflow", "watch", workspace, item] }

    final class Coordinator { var stream: TerminalStream? }
    func makeCoordinator() -> Coordinator { Coordinator() }

    /// Holds the terminal at the agent's own column count and scrolls when the window is narrower, so
    /// no part of the pane is cut off and the font stays readable.
    final class Scroller: NSScrollView {
        var terminal: TerminalRenderView?

        /// The pane is a fixed grid, so the window is often larger than it. The rest is the terminal's own
        /// background, so the picture reads as reaching the edges, and the scrollers are the always-visible
        /// kind so a pane wider than the window shows there is more to the right.
        init() {
            super.init(frame: .zero)
            hasHorizontalScroller = true
            hasVerticalScroller = true
            scrollerStyle = .legacy
            autohidesScrollers = false
            drawsBackground = true
            backgroundColor = Palette.background
        }
        required init?(coder: NSCoder) { fatalError("not used") }
        /// Size the document to exactly the pane's grid, never to the window. A larger view would ask
        /// the emulator for more columns than the agent's pane has, and the picture would be cleared.
        func fitDocument() {
            guard let terminal else { return }
            let size = terminal.contentSize
            if size.width > 0, terminal.frame.size != size { terminal.setFrameSize(size) }
        }
        override func tile() { super.tile(); fitDocument() }
    }

    func makeNSView(context: Context) -> Scroller {
        let scroller = Scroller()
        let view = makeTerminal(context: context)
        scroller.terminal = view
        scroller.documentView = view
        return scroller
    }

    private func makeTerminal(context: Context) -> TerminalRenderView {
        let view = TerminalRenderView()
        Self.makeReadOnly(view, item: item)
        view.setPaneGrid(nil)
        // The stream's first bytes are the pane's size, as a runner's marker.
        view.trustStreamSizes(true)
        view.beginLiveCore()
        let box = MainActorBox(view)
        let (seen, over, left) = (onBytes, onEnd, onExit)
        let stream = TerminalStream(onBytes: { bytes in
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    box.value.feed(bytes)
                    (box.value.enclosingScrollView as? Scroller)?.fitDocument()
                    seen()
                }
            }
        }, onEnd: { DispatchQueue.main.async { MainActor.assumeIsolated { over() } } },
        onExit: { code in DispatchQueue.main.async { MainActor.assumeIsolated { left(code) } } })
        if let binary {
            stream.start(binary: binary, terminal: "", environment: environment, host: hostArguments, command: command)
        }
        context.coordinator.stream = stream
        return view
    }

    func updateNSView(_ view: Scroller, context: Context) {}

    static func dismantleNSView(_ view: Scroller, coordinator: Coordinator) {
        coordinator.stream?.stop()
        coordinator.stream = nil
    }

    /// Nothing leaves this view toward the agent: the three ways out are closed here, in one place.
    @MainActor
    static func makeReadOnly(_ view: TerminalRenderView, item: String = "") {
        view.setAccessibilityLabel("Agent terminal for \(item), read-only")
        view.onInput = nil
        view.onPasteImage = nil
        view.onGeometry = nil
        view.takesKeyboard = false
        view.readOnly = true
    }
}
