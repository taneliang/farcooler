import SwiftUI

// What the conversation view draws for a turn claude wrote for itself
// (ov-474): a subagent's report, another session's message, a background
// task's notice and the harness's reminders. None is the person's message, so
// none is a bubble, and none shows XML. The projector read them
// (`harness.rs`); this only draws what it says, as the Mac's does
// (`NativeHarnessRows.swift` there).

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
            .foregroundStyle(.secondary)
            .lineLimit(2)
            .truncationMode(.middle)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityIdentifier("native-harness-status")
    }
}

/// A subagent's report: "Report from <what it was asked>", its words under
/// that folded after a few lines, and its transcript one tap away when the
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
                            .foregroundStyle(.tertiary)
                            .accessibilityHidden(true)
                    }
                    .frame(minHeight: 44, alignment: .leading)
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityHint("Opens this agent’s conversation")
                .accessibilityIdentifier("native-harness-open")
            } else {
                heading
            }
            if !harness.body.isEmpty {
                FoldedText(text: harness.body, lines: AgentConversation.harnessLines, markdown: true)
                    .font(.callout)
            }
        }
        .padding(.leading, Spacing.inset)
        .overlay(alignment: .leading) { Rectangle().fill(.quaternary).frame(width: 2) }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-harness-report")
    }

    private var heading: some View {
        Text(AgentConversation.harnessTitle(harness))
            .font(.caption.weight(.medium))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .truncationMode(.tail)
    }
}

/// The harness's reminders: one muted row, closed until asked for. They are
/// for the agent; a person rarely needs them, but can read what it was told.
private struct HarnessNote: View {
    let harness: AgentRow.Harness
    @State private var open = false

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Button { withAnimation(.snappy) { open.toggle() } } label: {
                HStack(spacing: Spacing.tight) {
                    Image(systemName: open ? "chevron.down" : "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .accessibilityHidden(true)
                    Text(AgentConversation.harnessTitle(harness))
                }
                .frame(minHeight: 44, alignment: .leading)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityValue(open ? "Expanded" : "Collapsed")
            .accessibilityIdentifier("native-harness-note-toggle")
            if open {
                Text(harness.body)
                    .textSelection(.enabled)
                    .padding(.leading, 14)
                    .accessibilityIdentifier("native-harness-note-body")
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-harness-note")
    }
}
