import AgentKit
import SwiftUI

/// Over a subagent's own conversation (ov-453): the way back, ⌘[ as in any
/// Mac app that goes back, and which agent this is.
struct NativeAgentHeader: View {
    let subagent: AgentRow.Subagent?
    let isFocused: Bool
    let back: () -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Button(action: back) {
                Label("Back", systemImage: "chevron.backward")
            }
            .keyboardShortcut(isFocused ? KeyboardShortcut("[", modifiers: .command) : nil)
            .help("Back to the Conversation (⌘[)")
            .identified("native-agent-back")
            if let subagent {
                NativeStatusMark(status: subagent.status)
                Text(AgentTray.agentType(subagent.agentType))
                    .fontWeight(.medium)
                    .fixedSize()
                Text(subagent.description)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .layoutPriority(-1)
                Spacer(minLength: Spacing.group)
                RunTimeChip(startedMs: subagent.startedMs, endedMs: subagent.status == .running ? nil : (subagent.endedMs ?? subagent.lastMs))
            } else {
                Spacer(minLength: 0)
            }
        }
        .font(.callout)
        .identified("native-agent-header")
    }
}
