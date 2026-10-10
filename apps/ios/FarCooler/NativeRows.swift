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
            case .thinking(let thinking): NativeThinkingRow(thinking: thinking)
            case .tool(let tool): NativeToolRow(id: row.id, tool: tool)
            case .subagent(let subagent): SubagentRow(subagent: subagent)
            case .ask(let ask): NativeAskRow(ask: ask, answer: answer, showTerminal: showTerminal)
            case .queued(let queued): QueuedLine(text: queued.text, state: queued.state, sendNow: sendNow)
            case .notice(let notice): NoticeLine(text: notice.text)
            case .handoff(let handoff): HandoffRow(reason: handoff.reason, showTerminal: showTerminal)
            case .gap(let gap): NoticeLine(text: AgentConversation.gap(gap))
            case .tasks(let tasks): NativeTasksRow(tasks: tasks)
            case .hint, .unknown: EmptyView()
            }
        }
        .opacity(row.provisional ? 0.75 : 1)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-row-\(row.id)")
    }
}

/// A run time that ticks without anything re-laying out: the system redraws
/// a timer `Text` itself, so no row's body is evaluated again (ov-382).
struct NativeRunTime: View {
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
            // the person's message, and a scheduled task's says it is one
            // (ov-452). Long ones fold.
            if AgentConversation.isScheduled(turn), !turn.prompt.isEmpty {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    Label(AgentConversation.scheduledTask, systemImage: "clock")
                        .font(.caption.weight(.medium))
                    FoldedText(text: turn.prompt, lines: 3).font(.callout)
                }
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("native-scheduled-turn")
            } else if AgentConversation.isNotice(turn), !turn.prompt.isEmpty {
                FoldedText(text: AgentConversation.noticeText(turn), lines: 3)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("native-notice-turn")
            } else if !turn.prompt.isEmpty {
                FoldedText(text: turn.prompt)
                    .padding(.horizontal, Spacing.inset)
                    .padding(.vertical, Spacing.group)
                    .surface(.inset, in: .card)
                    .padding(.leading, 48)
                    .accessibilityIdentifier("native-prompt")
            }
            // A notice's own turn says no time of its own: it's the line
            // above, not a message the person sent.
            if !AgentConversation.isNotice(turn), !AgentConversation.isScheduled(turn) { status }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
    }

    private var status: some View {
            HStack(spacing: Spacing.group) {
                if let note = AgentConversation.originNote(turn) { Text(note) }
                if turn.backgroundRunning > 0 {
                    Text(turn.backgroundRunning == 1 ? "1 agent still running" : "\(turn.backgroundRunning) agents still running")
                }
                if let outcome = AgentConversation.outcome(turn) {
                    Text(outcome)
                        .foregroundStyle(failed ? AnyShapeStyle(Tint.failure) : AnyShapeStyle(.secondary))
                } else {
                    HStack(spacing: Spacing.tight) {
                        ProgressView().controlSize(.mini)
                        NativeRunTime(startedMs: turn.startedMs, endedMs: nil)
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

struct NativeThinkingRow: View {
    let thinking: AgentRow.Thinking

    var body: some View {
        HStack(spacing: Spacing.tight) {
            Text(thinking.endedMs == nil ? "Thinking" : "Thought for")
            NativeRunTime(startedMs: thinking.startedMs, endedMs: thinking.endedMs)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}

/// A glyph for a tool's or a subagent's state. Color only where it needs
/// attention: a failure.
struct NativeStatusMark: View {
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

/// An edit's hunks, as unified-diff lines.
struct NativeDiff: View {
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
                NativeStatusMark(status: subagent.status)
                Text(AgentConversation.agentType(subagent.agentType)).fontWeight(.medium)
                Spacer(minLength: Spacing.group)
                Text(subagent.toolCount == 1 ? "1 tool" : "\(subagent.toolCount) tools")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                NativeRunTime(
                    startedMs: subagent.startedMs,
                    endedMs: subagent.status == .running ? nil : (subagent.endedMs ?? subagent.lastMs))
            }
            Text(subagent.description).lineLimit(2)
            if subagent.status == .running, !subagent.currentAction.isEmpty {
                Text(subagent.currentAction)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        // Unfilled and secondary, as a tool row is: the agent's machinery
        // sits below its words (ov-452).
        .font(MachineryStyle.font)
        .foregroundStyle(.secondary)
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
            FoldedText(text: text)
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
