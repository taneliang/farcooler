import SwiftUI

/// One row of the conversation view (ov-373), drawn from its own box, so a
/// change to this row re-renders this row and nothing else
/// (`AgentRowStore`). The same rows as the Mac's (`NativeRows.swift` there),
/// laid out for a phone's width.
struct NativeRowView: View {
    let box: AgentRowBox
    /// The newest row: prose there may still be growing, so it's drawn
    /// paragraph by paragraph and a flush re-lays out only the last one
    /// (`MarkdownText.streaming`, ov-382).
    var isLast = false
    let showTerminal: () -> Void
    /// A held ask's buttons (ov-370), where the runner takes answers.
    var answer: NativeAskAnswer?
    /// A waiting Queued row's Send Now (ov-368), where it's offered.
    var sendNow: (() -> Void)?

    var body: some View {
        let row = box.row
        Group {
            switch row.kind {
            case .turn(let turn): TurnRow(turn: turn)
            case .prose(let prose): AgentReplyText(text: prose.text, trailingClearance: 0, streaming: isLast)
            case .thinking(let thinking): ThinkingRow(thinking: thinking)
            case .tool(let tool): NativeToolRow(tool: tool)
            case .subagent(let subagent): SubagentRow(subagent: subagent)
            case .ask(let ask): NativeAskRow(ask: ask, answer: answer, showTerminal: showTerminal)
            case .queued(let queued): QueuedLine(text: queued.text, state: queued.state, sendNow: sendNow)
            case .notice(let notice): NoticeLine(text: notice.text)
            case .handoff(let handoff): HandoffRow(reason: handoff.reason, showTerminal: showTerminal)
            case .gap(let gap): NoticeLine(text: AgentConversation.gap(gap))
            case .unknown: EmptyView()
            }
        }
        .opacity(row.provisional ? 0.75 : 1)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-row-\(row.id)")
    }
}

/// A run time that ticks without anything re-laying out: the system redraws
/// a timer `Text` itself, so no row's body is evaluated again (ov-382).
private struct RunTimeChip: View {
    let startedMs: Int64?
    let endedMs: Int64?

    var body: some View {
        Group {
            if let startedMs, let endedMs {
                Text(AgentConversation.short(ms: endedMs - startedMs))
            } else if let startedMs {
                Text(
                    timerInterval: Date(timeIntervalSince1970: TimeInterval(startedMs) / 1000)...Date.distantFuture,
                    countsDown: false)
            }
        }
        .font(.caption.monospacedDigit())
        .foregroundStyle(.secondary)
        .fixedSize()
    }
}

private struct TurnRow: View {
    let turn: AgentRow.Turn

    var body: some View {
        VStack(alignment: .trailing, spacing: Spacing.tight) {
            // A turn whose prompt the projection never saw (a resume) has
            // only its outcome to show; one nobody typed is a notice, never
            // the person's message.
            if AgentConversation.isNotice(turn), !turn.prompt.isEmpty {
                Text(AgentConversation.noticeText(turn))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .accessibilityIdentifier("native-notice-turn")
            } else if !turn.prompt.isEmpty {
                Text(turn.prompt)
                    .textSelection(.enabled)
                    .padding(.horizontal, Spacing.inset)
                    .padding(.vertical, Spacing.group)
                    .surface(.inset, in: .card)
                    .padding(.leading, 48)
                    .accessibilityIdentifier("native-prompt")
            }
            // A notice's own turn says no time of its own: it's the line
            // above, not a message the person sent.
            if !AgentConversation.isNotice(turn) { status }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
    }

    private var status: some View {
            HStack(spacing: Spacing.group) {
                if turn.origin == "Queued" { Text("From the queue") }
                if turn.backgroundRunning > 0 {
                    Text(turn.backgroundRunning == 1 ? "1 agent still running" : "\(turn.backgroundRunning) agents still running")
                }
                if let outcome = AgentConversation.outcome(turn) {
                    Text(outcome)
                        .foregroundStyle(failed ? AnyShapeStyle(Tint.failure) : AnyShapeStyle(.secondary))
                } else {
                    HStack(spacing: Spacing.tight) {
                        ProgressView().controlSize(.mini)
                        RunTimeChip(startedMs: turn.startedMs, endedMs: nil)
                    }
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
    }

    private var failed: Bool {
        if case .failed? = turn.outcome { return true }
        return false
    }
}

private struct ThinkingRow: View {
    let thinking: AgentRow.Thinking

    var body: some View {
        HStack(spacing: Spacing.tight) {
            Text(thinking.endedMs == nil ? "Thinking" : "Thought for")
            RunTimeChip(startedMs: thinking.startedMs, endedMs: thinking.endedMs)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}

/// A glyph for a tool's or a subagent's state. Color only where it needs
/// attention: a failure.
private struct StatusMark: View {
    let status: AgentRow.Status

    var body: some View {
        Group {
            switch status {
            case .running: ProgressView().controlSize(.mini)
            case .done: Image(systemName: "checkmark").foregroundStyle(.secondary)
            case .failed: Image(systemName: "xmark").foregroundStyle(Tint.failure)
            case .ended: Image(systemName: "stop").foregroundStyle(.secondary)
            }
        }
        .frame(width: 16)
    }
}

private struct NativeToolRow: View {
    let tool: AgentRow.Tool
    @State private var open = false

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Button {
                if !tool.diff.isEmpty { open.toggle() }
            } label: {
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    StatusMark(status: tool.status)
                    Text(tool.name).fontWeight(.medium).foregroundStyle(.primary)
                    Text(tool.summary)
                        .font(.callout.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: Spacing.group)
                    if !tool.diff.isEmpty {
                        Image(systemName: open ? "chevron.up" : "chevron.down")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    RunTimeChip(startedMs: tool.startedMs, endedMs: tool.endedMs)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(tool.diff.isEmpty)
            .accessibilityHint(tool.diff.isEmpty ? "" : (open ? "Hides the diff" : "Shows the diff"))
            if open {
                NativeDiff(hunks: tool.diff)
            }
        }
        .font(.callout)
    }
}

/// An edit's hunks, as unified-diff lines.
private struct NativeDiff: View {
    let hunks: [AgentRow.Hunk]

    var body: some View {
        ScrollView(.horizontal) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(hunks.indices, id: \.self) { h in
                    ForEach(hunks[h].lines.indices, id: \.self) { l in
                        let line = hunks[h].lines[l]
                        Text(line.isEmpty ? " " : line)
                            .font(.caption.monospaced())
                            .foregroundStyle(line.hasPrefix("-") ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
                            .fixedSize()
                    }
                }
            }
            .textSelection(.enabled)
            .padding(Spacing.group)
        }
        .surface(.inset, in: .control)
    }
}

private struct SubagentRow: View {
    let subagent: AgentRow.Subagent

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                StatusMark(status: subagent.status)
                Text(AgentConversation.agentType(subagent.agentType)).fontWeight(.medium)
                Spacer(minLength: Spacing.group)
                Text(subagent.toolCount == 1 ? "1 tool" : "\(subagent.toolCount) tools")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                RunTimeChip(
                    startedMs: subagent.startedMs,
                    endedMs: subagent.status == .running ? nil : (subagent.endedMs ?? subagent.lastMs))
            }
            Text(subagent.description).foregroundStyle(.secondary).lineLimit(2)
            if subagent.status == .running, !subagent.currentAction.isEmpty {
                Text(subagent.currentAction)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .font(.callout)
        .padding(Spacing.inset)
        .surface(.inset, in: .card)
    }
}

/// A message in claude's own queue (R-29), from its transcript or sent from
/// here a moment ago.
struct QueuedLine: View {
    let text: String
    var state: String = "Waiting"
    /// Send Now (ov-368): claude takes every waiting message now, rather than
    /// when its turn ends. Offered on a waiting message only.
    var sendNow: (() -> Void)?

    var body: some View {
        VStack(alignment: .trailing, spacing: Spacing.tight) {
            Text(text)
                .padding(.horizontal, Spacing.inset)
                .padding(.vertical, Spacing.group)
                .surface(.inset, in: .card)
            HStack(spacing: Spacing.group) {
                Text(AgentConversation.queuedLabel(state))
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                if state == "Waiting", let sendNow {
                    Button("Send Now", action: sendNow)
                        .font(.caption.weight(.semibold))
                        // A 44 pt band around a caption's button: the
                        // guideline's floor, with no visible chrome.
                        .frame(minHeight: 44)
                        .accessibilityIdentifier("native-send-now")
                }
            }
        }
        .padding(.leading, 48)
        .frame(maxWidth: .infinity, alignment: .trailing)
        // Contained, not combined: Send Now is a button of its own.
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-queued")
    }
}

private struct NoticeLine: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.caption)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
    }
}

/// Something only the terminal can show: a panel, a dialog, a question
/// Claude is asking there.
struct HandoffRow: View {
    let reason: String
    let showTerminal: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                Image(systemName: "terminal")
                Text(reason)
            }
            Button("Show Terminal", action: showTerminal)
                .buttonStyle(.bordered)
                .accessibilityIdentifier("native-handoff-show-terminal")
        }
        .font(.callout)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(Spacing.inset)
        .attentionSurface(in: .card)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-handoff")
    }
}
