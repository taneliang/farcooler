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
    @State private var ended = false

    /// What to tell the owner when the stream stops: nothing while it runs, "finished" after a picture,
    /// and the reason when there never was one (a review, a merge, or an agent that already ended).
    static func status(gotBytes: Bool, ended: Bool) -> String? {
        guard ended else { return nil }
        return gotBytes ? "Finished" : "This agent isn’t running right now."
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Watching \(item)").font(.headline)
                Text("Read-only").font(.caption).foregroundStyle(.secondary)
                if gotBytes, let done = Self.status(gotBytes: gotBytes, ended: ended) {
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
                    onBytes: { gotBytes = true }, onEnd: { ended = true })
                    .overlay {
                        if let why = Self.status(gotBytes: gotBytes, ended: ended), !gotBytes {
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

    /// The whole of what this view ever asks the CLI to do. Pinned by a test.
    static func command(workspace: String, item: String) -> [String] { ["workflow", "watch", workspace, item] }

    final class Coordinator { var stream: TerminalStream? }
    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> TerminalRenderView {
        let view = TerminalRenderView()
        Self.makeReadOnly(view, item: item)
        view.setPaneGrid(nil)
        // The stream's first bytes are the pane's size, as a runner's marker.
        view.trustStreamSizes(true)
        view.beginLiveCore()
        let box = MainActorBox(view)
        let (seen, over) = (onBytes, onEnd)
        let stream = TerminalStream(onBytes: { bytes in
            DispatchQueue.main.async { MainActor.assumeIsolated { box.value.feed(bytes); seen() } }
        }, onEnd: { DispatchQueue.main.async { MainActor.assumeIsolated { over() } } })
        if let binary {
            stream.start(binary: binary, terminal: "", environment: environment, host: hostArguments, command: command)
        }
        context.coordinator.stream = stream
        return view
    }

    func updateNSView(_ view: TerminalRenderView, context: Context) {}

    static func dismantleNSView(_ view: TerminalRenderView, coordinator: Coordinator) {
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
