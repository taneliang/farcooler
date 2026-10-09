import AgentKit
import SwiftUI

/// The pane's switch where the conversation view isn't offered, for a reason
/// about the runner, the setting or the pairing (ov-443): the same glyph,
/// dimmed, in the same place, and a click says why, with the way to Settings
/// where that's where it's fixed. A pane that isn't Claude or Codex shows no
/// switch at all (`AgentConversation.Unavailable.isAboutTheRunner`).
struct ConversationUnavailableChip: View {
    let reason: AgentConversation.Unavailable
    @State private var explaining = false
    @Environment(\.openSettings) private var openSettings

    /// The Settings tab that fixes `reason`, if one does.
    static func settingsTab(for reason: AgentConversation.Unavailable) -> String? {
        switch reason {
        case .settingOff: return SettingsTab.general
        case .pairingNeeded: return "devices"
        default: return nil
        }
    }

    var body: some View {
        Button {
            explaining.toggle()
        } label: {
            Image(systemName: "text.bubble")
                .foregroundStyle(.tertiary)
                .frame(width: 16, height: 16)
        }
        .buttonStyle(.borderless)
        .padding(Spacing.group)
        .surface(.content, in: .floating)
        .padding(Spacing.group)
        .help("Conversation Unavailable: \(reason.sentence)")
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

/// What the chip's popover says: the reason, and Open Settings where
/// Settings is where it's fixed.
struct ConversationUnavailableNote: View {
    let reason: AgentConversation.Unavailable
    let open: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            Text("Conversation Unavailable").font(.headline)
            Text(reason.sentence)
                .foregroundStyle(.secondary)
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
