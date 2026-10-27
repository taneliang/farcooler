import AgentKit
import SwiftUI

/// The pane's switch where the conversation view isn't offered, for a reason
/// about the runner or the pairing (ov-443): the same glyph,
/// dimmed, in the same place, and a click says why, with the way to Settings
/// where that's where it's fixed. A pane that isn't Claude or Codex shows no
/// switch at all (`AgentConversation.Unavailable.isAboutTheRunner`).
struct ConversationUnavailableChip: View {
    let reason: AgentConversation.Unavailable
    /// Beside the agents in the orchestrator's corner (ov-829): the corner's
    /// size and glass, the group's inset.
    var inGroup = false
    @State private var explaining = false
    @Environment(\.openSettings) private var openSettings

    /// The Settings tab that fixes `reason`, if one does.
    static func settingsTab(for reason: AgentConversation.Unavailable) -> String? {
        switch reason {
        case .pairingNeeded: return "devices"
        default: return nil
        }
    }

    var body: some View {
        chip
        .help("Conversation Unavailable. \(reason.note)")
        .accessibilityLabel("Conversation Unavailable")
        .accessibilityValue(reason.sentence)
        .identified("native-switch-unavailable")
        .popover(isPresented: $explaining, arrowEdge: .bottom) {
            ConversationUnavailableNote(reason: reason) { tab in
                explaining = false
                Preferences.shared.settingsTab = tab
                openSettings()
            }
        }
    }
}

extension ConversationUnavailableChip {
    @ViewBuilder fileprivate var chip: some View {
        if inGroup {
            Button {
                explaining.toggle()
            } label: {
                Image(systemName: "text.bubble")
                    .font(.system(size: 12))
                    .foregroundStyle(.quietInk)
                    .frame(width: PaneCorner.control, height: PaneCorner.control)
                    .contentShape(PaneCorner.shape)
            }
            .buttonStyle(.plain)
            .surface(.floating, in: PaneCorner.shape, interactive: true)
        } else {
            Button {
                explaining.toggle()
            } label: {
                Image(systemName: "text.bubble")
                    .foregroundStyle(.quietInk)
                    .frame(width: 16, height: 16)
            }
            .buttonStyle(.borderless)
            .padding(Spacing.group)
            .surface(.content, in: .floating)
            .padding(Spacing.group)
        }
    }
}

/// What the chip's popover says: the reason, and Open Settings where
/// Settings is where it's fixed.
struct ConversationUnavailableNote: View {
    let reason: AgentConversation.Unavailable
    let open: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            Text("Conversation Unavailable").font(.headline)
            Text(reason.note)
                .foregroundStyle(.secondary)  // style-exempt: a popover's text, system material
                .fixedSize(horizontal: false, vertical: true)
            if let tab = ConversationUnavailableChip.settingsTab(for: reason) {
                Button("Open Settings…") { open(tab) }
                    .identified("native-unavailable-settings")
            }
        }
        .padding(Spacing.section)
        .frame(width: 260, alignment: .leading)
    }
}
