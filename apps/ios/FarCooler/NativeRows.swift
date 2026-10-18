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
            case .turn(let turn): TurnRow(id: row.id, turn: turn)
            case .prose(let prose): AgentReplyText(text: prose.text, trailingClearance: 0, streaming: isLast)
            case .thinking(let thinking): NativeThinkingRow(thinking: thinking)
            case .tool(let tool): NativeToolRow(id: row.id, tool: tool)
            case .subagent(let subagent): SubagentRow(id: row.id, subagent: subagent)
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
    let id: String
    let turn: AgentRow.Turn
    @Environment(\.promptImages) private var store

    var body: some View {
        if let store {
            Served(id: id, turn: turn, store: store)
        } else {
            TurnBody(id: id, turn: turn)
        }
    }

    /// Observes the store, so a runner that comes to serve images (or stops)
    /// redraws the turn.
    private struct Served: View {
        let id: String
        let turn: AgentRow.Turn
        @ObservedObject var store: PromptImageStore

        var body: some View {
            // A thumbnail drawn stands for its `[Image #N]`, which leaves the
            // words (ov-454); where none is drawn the words keep it.
            var shown = turn
            if store.source != nil {
                shown.prompt = AgentConversation.withoutPlaceholders(turn.prompt, shown: turn.images.count)
            }
            return TurnBody(id: id, turn: shown, images: turn.images)
        }
    }
}

private struct TurnBody: View {
    let id: String
    let turn: AgentRow.Turn
    var images: [AgentRow.PromptImage] = []

    var body: some View {
        VStack(alignment: .trailing, spacing: Spacing.tight) {
            PromptImageStrip(row: id, images: images)
            // A turn whose prompt the projection never saw (a resume) has
            // only its outcome to show; one nobody typed is a notice, never
            // the person's message, and a scheduled task's says it is one
            // (ov-452). Long ones fold.
            if let harness = turn.harness {
                // A report, a task's notice or a reminder (ov-474): its own
                // row, with no outcome line of its own.
                HarnessTurn(harness: harness)
            } else if AgentConversation.isScheduled(turn), !turn.prompt.isEmpty {
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
                YourBubble(text: turn.prompt)
                    .accessibilityIdentifier("native-prompt")
            }
            // A notice's own turn says no time of its own: it's the line
            // above, not a message the person sent. Nor does a report's.
            if !AgentConversation.isNotice(turn), !AgentConversation.isScheduled(turn), !AgentConversation.isHarness(turn) { status }
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
///
/// Every state is drawn in one frame, the height of a line of the row's own
/// text and 16 points wide, centered on it, and that frame carries the
/// text's baseline: a spinner has no baseline of its own, so on the row's
/// first-baseline line it sat on the baseline, below the words, and the row
/// jumped when it finished and became a checkmark (ov-611).
struct NativeStatusMark: View {
    let status: AgentRow.Status
    /// 16 points at the default size, growing with the type, so a large
    /// checkmark stays inside its frame and the margin (review 1).
    @ScaledMetric(relativeTo: .callout) private var side: CGFloat = 16

    var body: some View {
        Text(verbatim: "M")
            .hidden()
            .frame(width: side)
            .overlay {
                switch status {
                case .running: ProgressView().controlSize(.mini)
                case .done: Image(systemName: "checkmark").foregroundStyle(.secondary)
                case .failed: Image(systemName: "xmark").foregroundStyle(Tint.failure)
                case .ended: Image(systemName: "stop").foregroundStyle(.secondary)
                }
            }
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
    let id: String
    let subagent: AgentRow.Subagent
    @Environment(\.nativeOpenAgent) private var openAgent

    var body: some View {
        if let openAgent, let agentId = subagent.agentId {
            // Opens to its own conversation (ov-453).
            Button { openAgent(id, agentId) } label: {
                content(opens: true).contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityHint("Opens this agent’s conversation")
            .accessibilityIdentifier("native-subagent-open-\(id)")
        } else {
            content(opens: false)
        }
    }

    private func content(opens: Bool) -> some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                NativeStatusMark(status: subagent.status)
                Text(AgentConversation.agentType(subagent.agentType)).fontWeight(.medium)
                Spacer(minLength: Spacing.group)
                if let tools = AgentConversation.toolsLabel(subagent.toolCount) {
                    Text(tools)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("native-subagent-tools")
                }
                NativeRunTime(
                    startedMs: subagent.startedMs,
                    endedMs: subagent.status == .running ? nil : (subagent.endedMs ?? subagent.lastMs))
                if opens {
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.tertiary)
                        .accessibilityHidden(true)
                }
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
            YourBubble(text: text)
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

/// The person's own message (ov-610): the site's blue bubble, white text,
/// at the trailing edge and as wide as its words up to three quarters of the
/// column, with the small corner at the bottom trailing as its tail. Its
/// words select ranges in white (ov-609), and Show More reads on the blue.
struct YourBubble: View {
    let text: String
    @Environment(\.colorScheme) private var scheme
    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        TrailingMeasure() {
            FoldedText(text: text)
                .environment(\.selectableColor, .white)
                .foregroundStyle(MessageBubble.text)
                .padding(.horizontal, Spacing.inset)
                .padding(.vertical, Spacing.group)
                .background(MessageBubble.fill(scheme, contrast: contrast), in: MessageBubble.shape)
        }
    }
}
