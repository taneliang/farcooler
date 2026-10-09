import AgentKit
import SwiftUI

/// A terminal pane with its native view beside it (ov-372): both mounted,
/// one shown, switched on the client alone.
///
/// The terminal is always the first child and never leaves the tree, so a
/// switch never respawns the pane, never restarts its stream and works
/// mid-turn and mid-dialog; the native view keeps its draft and rows in its
/// `NativePaneModel`, which outlives this view. The terminal shows wherever
/// the native view isn't offered (`NativeAgents.offers`): never a blank pane.
/// There a Claude or Codex pane's switch is dimmed and says why
/// (`ConversationUnavailableChip`, ov-443); any other pane has none.
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
    /// This mount's say in whether the pane is on screen (`NativePaneModel.onScreen`).
    @State private var mount = UUID()
    /// Not behind a zoomed pane or an unselected conversation's own layer
    /// (`outOfSight`), and in a window somebody can see (`windowVisible`).
    @Environment(\.outOfSight) private var outOfSight
    @Environment(\.windowVisible) private var windowVisible

    var body: some View {
        let model = agents.offers(terminal, target: target)
            ? agents.model(for: terminal.id, target: target, program: NativeAgents.agent(of: terminal)) : nil
        // Where it isn't offered, a Claude or Codex pane says why (ov-443).
        let reason = model == nil ? NativeSwitchReason.shown(agents.unavailable(terminal, target: target)) : nil
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
            } else if let reason {
                ConversationUnavailableChip(reason: reason)
            }
        }
        .onDisappear { agents.model(ifMade: terminal.id)?.setOnScreen(false, by: mount) }
        .task(id: model.map(ObjectIdentifier.init)) {
            guard let model else {
                native = false
                return
            }
            // Says whether this pane is on screen, then follows if it should:
            // a model whose view came back, as the runner started serving
            // rows again, restarts a follow that ended `.unavailable`.
            model.setOnScreen(!outOfSight && windowVisible, by: mount)
            model.followIfShown()
            for await value in model.$showsNative.values { native = value }
        }
        .onChange(of: !outOfSight && windowVisible) { _, visible in
            model?.setOnScreen(visible, by: mount)
        }
        
    }
}

/// Which reasons a pane's dimmed switch shows (ov-443): one about the runner
/// or the pairing. Not the setting being off, which is a choice: with it off
/// by default, a dimmed switch on every Claude pane would be noise. The menu
/// item's help still says it.
enum NativeSwitchReason {
    static func shown(_ reason: AgentConversation.Unavailable?) -> AgentConversation.Unavailable? {
        guard let reason, reason.isAboutTheRunner, reason != .settingOff else { return nil }
        return reason
    }
}
