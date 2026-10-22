import AgentKit
import SwiftUI

// What the native view draws for a turn claude wrote for itself (ov-474): a
// subagent's report, another session's message, a background task's notice
// and the harness's reminders. None is the person's message, so none is a
// bubble, and none shows XML. The projector read them (`harness.rs`); this
// only draws what it says.

/// One such turn, by kind: a report or message under its heading, a status
/// as one muted line, a reminder folded into a System note.
struct HarnessTurn: View {
    let harness: AgentRow.Harness

    var body: some View {
        switch harness.kind {
        case "Status": HarnessStatus(title: AgentConversation.harnessTitle(harness))
        case "Note": HarnessNote(harness: harness)
        default: HarnessReport(harness: harness)
        }
    }
}

/// A background task finished: one muted line, as claude's own notices are.
private struct HarnessStatus: View {
    let title: String

    var body: some View {
        Text(title)
            .font(.caption)
            .foregroundStyle(.quietInk)
            .lineLimit(2)
            .truncationMode(.middle)
            .frame(maxWidth: .infinity, alignment: .leading)
            .identified("native-harness-status")
    }
}

/// A subagent's report: "Report from <what it was asked>", its words under
/// that folded after a few lines, and its transcript one click away when the
/// runner can open it (ov-453).
private struct HarnessReport: View {
    let harness: AgentRow.Harness
    @Environment(\.nativeOpenAgent) private var openAgent

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            if let openAgent, let agent = AgentConversation.harnessAgent(harness) {
                Button { openAgent(agent.row, agent.agentId) } label: {
                    HStack(spacing: Spacing.tight) {
                        heading
                        Image(systemName: "chevron.right")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.quietInk)
                            .accessibilityHidden(true)
                    }
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help("Open This Agent’s Conversation")
                .accessibilityHint("Opens this agent’s conversation")
                .identified("native-harness-open")
            } else {
                heading
            }
            if !harness.body.isEmpty {
                FoldedMarkdown(text: harness.body, lines: AgentConversation.harnessLines)
            }
        }
        .padding(.leading, Spacing.inset)
        .overlay(alignment: .leading) { Rectangle().fill(.quaternary).frame(width: 2) }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .identified("native-harness-report")
    }

    private var heading: some View {
        Text(AgentConversation.harnessTitle(harness))
            .font(.caption.weight(.medium))
            .foregroundStyle(.quietInk)
            .lineLimit(1)
            .truncationMode(.tail)
    }
}

/// The harness's reminders: one muted row, closed until asked for. They are
/// for the agent; a person rarely needs them, but can read what it was told.
private struct HarnessNote: View {
    let harness: AgentRow.Harness
    @State private var open = false
    @Environment(\.selectsText) private var selectsText

    var body: some View {
        CollapsibleSection(
            id: "harness-note", metrics: .inline, isExpanded: $open, canExpand: true,
            accessibilityLabel: AgentConversation.harnessTitle(harness),
            label: { _ in
                Text(AgentConversation.harnessTitle(harness))
                    .font(.caption)
                    .foregroundStyle(.quietInk)
            },
            accessory: { EmptyView() }
        ) {
            Text(harness.body)
                .font(.caption)
                .foregroundStyle(.quietInk)
                .selectable(selectsText)
                .frame(maxWidth: .infinity, alignment: .leading)
                .identified("native-harness-note-body")
        }
        .identified("native-harness-note")
    }
}
