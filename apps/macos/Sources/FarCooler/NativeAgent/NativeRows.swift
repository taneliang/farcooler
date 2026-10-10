import AgentKit
import SwiftUI

/// One row of the native view (ov-372), drawn from its own box, so a change
/// to this row re-renders this row and nothing else (`AgentRowStore`).
struct NativeRowView: View {
    let box: AgentRowBox
    /// The newest row: prose there may still be growing, so it's drawn
    /// paragraph by paragraph and a flush re-lays out only the last one
    /// (`MarkdownText.streaming`, ov-382).
    var isLast = false
    let showTerminal: () -> Void
    /// A waiting Queued row's Send Now (ov-368), where it's offered.
    var sendNow: (() -> Void)?
    /// A held ask's buttons (ov-370), where the runner takes answers.
    var answer: NativeAnswer?

    var body: some View {
        let row = box.row
        Group {
            switch row.kind {
            case .turn(let turn):
                // Its images above it, in their own view (ov-454).
                VStack(alignment: .trailing, spacing: Spacing.tight) {
                    PromptImageStrip(row: row.id, images: turn.images)
                    TurnRow(turn: turn)
                }
            case .prose(let prose):
                // Selectable once settled: `MarkdownPiece` turns selection
                // on for the pieces that are done.
                AgentReplyText(text: prose.text, trailingClearance: 0, streaming: isLast)
            case .thinking(let thinking): NativeThinkingRow(thinking: thinking)
            case .tool(let tool): NativeToolRow(id: row.id, tool: tool)
            case .subagent(let subagent): SubagentRow(subagent: subagent)
            case .ask(let ask): NativeAskRow(ask: ask, answer: answer, showTerminal: showTerminal)
            case .queued(let queued): QueuedLine(text: queued.text, state: queued.state, sendNow: sendNow)
            case .notice(let notice): NoticeLine(text: notice.text)
            case .handoff(let handoff): HandoffRow(reason: handoff.reason, showTerminal: showTerminal)
            case .gap(let gap): NoticeLine(text: NativeCopy.gap(gap))
            case .tasks(let tasks): NativeTasksRow(tasks: tasks)
            case .hint, .unknown: EmptyView()
            }
        }
        .opacity(row.provisional ? 0.75 : 1)
        .identified("native-row-\(row.id)")
    }
}

/// The words the native view uses, in one place.
enum NativeCopy {
    static func gap(_ gap: AgentRow.Gap) -> String {
        gap.count > 1 ? "Some of this session couldn’t be read." : "A line of this session couldn’t be read."
    }

    /// "0:04", "1:12", "2:03:09": the format a running timer counts in, so
    /// a finished time reads like the running one did.
    static func short(ms: Int64) -> String {
        let seconds = max(0, ms / 1000)
        let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60)
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%d:%02d", m, s)
    }

    /// A subagent's type as words: `general-purpose` reads "General purpose".
    static func agentType(_ raw: String) -> String {
        let words = raw.replacingOccurrences(of: "-", with: " ").replacingOccurrences(of: "_", with: " ")
        guard let first = words.first else { return "Agent" }
        return first.uppercased() + words.dropFirst()
    }

    /// A turn nobody typed: a background task finishing, or claude waking
    /// itself (`TurnOrigin::Notification`, `System`). Drawn as a notice,
    /// never as the person's message.
    static func isNotice(_ turn: AgentRow.Turn) -> Bool {
        turn.origin == "Notification" || turn.origin == "System"
    }

    /// A tool's or a subagent's state, as VoiceOver says it.
    static func status(_ status: AgentRow.Status) -> String {
        switch status {
        case .running: "Running"
        case .done: "Done"
        case .failed: "Failed"
        case .ended: "Ended"
        }
    }

    static func outcome(_ turn: AgentRow.Turn) -> String? {
        switch turn.outcome {
        case nil: nil
        case .finished?: turn.durationMs.map { "Took \(short(ms: $0))" } ?? "Done"
        case .interrupted?: "Interrupted"
        case .unrecorded?: "Not recorded"
        case .failed(let detail)?: detail.isEmpty ? "Failed" : "Failed: \(detail)"
        case .other?: "Ended"
        }
    }
}

/// A run time that ticks without anything re-laying out: the system redraws
/// a timer `Text` itself, so no `TimelineView` runs at display rate and no
/// row's body is evaluated again (the ShimmerBand lesson, ov-382). Fixed
/// width digits, so a tick never changes the row's size.
struct RunTimeChip: View {
    let startedMs: Int64?
    let endedMs: Int64?

    var body: some View {
        Group {
            if let startedMs, let endedMs {
                Text(NativeCopy.short(ms: endedMs - startedMs))
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
            // only its outcome to show; one nobody typed is a notice, and a
            // scheduled task's says it is one (ov-452). Long ones fold.
            if AgentConversation.isScheduled(turn), !turn.prompt.isEmpty {
                ScheduledPrompt(prompt: turn.prompt)
            } else if NativeCopy.isNotice(turn), !turn.prompt.isEmpty {
                FoldedText(text: AgentConversation.noticeText(turn), lines: 3)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .identified("native-notice-turn")
            } else if !turn.prompt.isEmpty {
                FoldedText(text: turn.prompt)
                    .padding(.horizontal, Spacing.inset)
                    .padding(.vertical, Spacing.group)
                    .surface(.inset, in: .card)
                    .frame(maxWidth: 560, alignment: .trailing)
            }
            HStack(spacing: Spacing.group) {
                if let note = AgentConversation.originNote(turn) { Text(note) }
                if turn.backgroundRunning > 0 {
                    Text(turn.backgroundRunning == 1 ? "1 agent still running" : "\(turn.backgroundRunning) agents still running")
                }
                if let outcome = NativeCopy.outcome(turn) {
                    Text(outcome)
                        .foregroundStyle(failed ? AnyShapeStyle(Tint.failure) : AnyShapeStyle(.secondary))
                } else {
                    Label { RunTimeChip(startedMs: turn.startedMs, endedMs: nil) } icon: { ProgressView().controlSize(.mini) }
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
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
            RunTimeChip(startedMs: thinking.startedMs, endedMs: thinking.endedMs)
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
        switch status {
        case .running: ProgressView().controlSize(.mini).frame(width: 14)
        case .done: Image(systemName: "checkmark").foregroundStyle(.secondary).frame(width: 14)
        case .failed: Image(systemName: "xmark").foregroundStyle(Tint.failure).frame(width: 14)
        case .ended: Image(systemName: "stop").foregroundStyle(.secondary).frame(width: 14)
        }
    }
}

private struct SubagentRow: View {
    let subagent: AgentRow.Subagent

    var body: some View {
        // Unfilled and secondary, as a tool row is: the agent's machinery
        // sits below its words (ov-452).
        VStack(alignment: .leading, spacing: Spacing.tight) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                NativeStatusMark(status: subagent.status)
                Text(NativeCopy.agentType(subagent.agentType)).fontWeight(.medium)
                Text(subagent.description).lineLimit(1).layoutPriority(-1)
                Spacer(minLength: Spacing.group)
                Text(subagent.toolCount == 1 ? "1 tool" : "\(subagent.toolCount) tools")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                RunTimeChip(startedMs: subagent.startedMs, endedMs: subagent.status == .running ? nil : (subagent.endedMs ?? subagent.lastMs))
            }
            if subagent.status == .running, !subagent.currentAction.isEmpty {
                Text(subagent.currentAction)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .padding(.leading, 14 + Spacing.group)
            }
        }
        .font(MachineryStyle.font)
        .foregroundStyle(.secondary)
        .padding(.leading, 14)
    }
}

/// A scheduled task's prompt (ov-452): said to be one, its words folded to a
/// few lines, at the leading edge where claude's own notices sit, since
/// nobody typed it.
private struct ScheduledPrompt: View {
    let prompt: String

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Label(AgentConversation.scheduledTask, systemImage: "clock")
                .font(.caption.weight(.medium))
            FoldedText(text: prompt, lines: 3)
                .font(.callout)
        }
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .identified("native-scheduled-turn")
    }
}

/// A message in claude's own queue (R-29), from its transcript or sent from
/// here a moment ago.
struct QueuedLine: View {
    let text: String
    var state: String = "Waiting"
    /// Send Now (ov-368): claude takes every waiting message now, rather than
    /// when its turn ends. Only on a message still waiting, where offered.
    var sendNow: (() -> Void)?

    var body: some View {
        // The person's message, as a prompt is drawn, with what became of
        // it under it: still queued, taken into the running turn, or taken
        // back (ov-452).
        VStack(alignment: .trailing, spacing: Spacing.tight) {
            FoldedText(text: text)
                .padding(.horizontal, Spacing.inset)
                .padding(.vertical, Spacing.group)
                .surface(.inset, in: .card)
                .frame(maxWidth: 560, alignment: .trailing)
            HStack(spacing: Spacing.group) {
                Text(AgentConversation.queuedLabel(state))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if state == "Waiting", let sendNow {
                    Button("Send Now", action: sendNow)
                        .controlSize(.small)
                        .help("Claude reads the queued messages now instead of after this turn.")
                        .identified("native-send-now")
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
        .identified("native-queued")
    }
}

private struct NoticeLine: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.caption)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity)
    }
}

/// Something only the terminal can show: a panel, a dialog, a question
/// Claude is asking there.
struct HandoffRow: View {
    let reason: String
    let showTerminal: () -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Image(systemName: "terminal")
            Text(reason)
            Spacer(minLength: Spacing.group)
            Button("Show Terminal", action: showTerminal)
                .identified("native-handoff-show-terminal")
        }
        .font(.callout)
        .padding(Spacing.group)
        .attentionSurface(in: .card)
        .identified("native-handoff")
    }
}
