import AgentKit
import SwiftUI

/// A terminal pane with its native view beside it (ov-372): both mounted,
/// one shown, switched on the client alone.
///
/// The terminal is always the first child and never leaves the tree, so a
/// switch never respawns the pane, never restarts its stream and works
/// mid-turn and mid-dialog; the native view keeps its draft and rows in its
/// `NativePaneModel`, which outlives this view. The switch is hidden, and
/// the terminal shows, wherever the native view isn't offered
/// (`NativeAgents.offers`): never a blank pane.
struct NativeSwitch<Surface: View>: View {
    let terminal: Terminal
    /// The runner, as `DaemonClient.target` names it: empty for this Mac's.
    let target: String
    let isFocused: Bool
    /// The terminal surface, told whether it holds the keyboard.
    @ViewBuilder let surface: (Bool) -> Surface
    @ObservedObject private var agents: NativeAgents

    init(
        terminal: Terminal, target: String, isFocused: Bool, agents: NativeAgents = .shared,
        @ViewBuilder surface: @escaping (Bool) -> Surface
    ) {
        self.terminal = terminal
        self.target = target
        self.isFocused = isFocused
        self.surface = surface
        _agents = ObservedObject(wrappedValue: agents)
    }

    /// The model's `showsNative`, mirrored so this view, which holds the
    /// terminal, redraws when it changes.
    @State private var native = false

    var body: some View {
        let model = agents.offers(terminal, target: target) ? agents.model(for: terminal.id) : nil
        let showing = native && model != nil
        // The terminal first, always: the native layer coming and going
        // leaves its identity, and so its stream, alone.
        ZStack {
            surface(isFocused && !showing)
            if let model {
                NativeAgentView(model: model, isFocused: isFocused && showing, showTerminal: { model.showsNative = false })
                    .opacity(showing ? 1 : 0)
                    .allowsHitTesting(showing)
                    .accessibilityHidden(!showing)
            }
        }
        .overlay(alignment: .topTrailing) {
            if let model {
                Button {
                    model.showsNative.toggle()
                } label: {
                    Image(systemName: showing ? "terminal" : "text.bubble")
                        .foregroundStyle(.primary)
                        .frame(width: 16, height: 16)
                }
                .buttonStyle(.borderless)
                .padding(Spacing.group)
                // Opaque paper, so the glyph reads over a dark terminal in a
                // light window as well as over the conversation.
                .surface(.content, in: .floating)
                .padding(Spacing.group)
                .help(showing ? "Show Terminal" : "Show Conversation")
                .accessibilityLabel(showing ? "Show Terminal" : "Show Conversation")
                .identified("native-switch")
            }
        }
        .task(id: model.map(ObjectIdentifier.init)) {
            guard let model else {
                native = false
                return
            }
            for await value in model.$showsNative.values { native = value }
        }
    }
}
