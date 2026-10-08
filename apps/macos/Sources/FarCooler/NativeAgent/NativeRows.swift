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
            case .turn(let turn): TurnRow(turn: turn)
            case .prose(let prose):
                // Selectable once settled: `MarkdownPiece` turns selection
                // on for the pieces that are done.
                AgentReplyText(text: prose.text, trailingClearance: 0, streaming: isLast)
            case .thinking(let thinking): ThinkingRow(thinking: thinking)
            case .tool(let tool): ToolRow(tool: tool)
            case .subagent(let subagent): SubagentRow(subagent: subagent)
            case .ask(let ask): NativeAskRow(ask: ask, answer: answer, showTerminal: showTerminal)
            case .queued(let queued): QueuedLine(text: queued.text, state: queued.state, sendNow: sendNow)
            case .notice(let notice): NoticeLine(text: notice.text)
            case .handoff(let handoff): HandoffRow(reason: handoff.reason, showTerminal: showTerminal)
            case .gap(let gap): NoticeLine(text: NativeCopy.gap(gap))
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
            // only its outcome to show; one nobody typed is a notice.
            if NativeCopy.isNotice(turn), !turn.prompt.isEmpty {
                Text(turn.prompt.replacingOccurrences(of: "\"", with: ""))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .identified("native-notice-turn")
            } else if !turn.prompt.isEmpty {
                Text(turn.prompt)
                    .textSelection(.enabled)
                    .padding(.horizontal, Spacing.inset)
                    .padding(.vertical, Spacing.group)
                    .surface(.inset, in: .card)
                    .frame(maxWidth: 560, alignment: .trailing)
            }
            HStack(spacing: Spacing.group) {
                if turn.origin == "Queued" { Text("From the queue") }
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
        switch status {
        case .running: ProgressView().controlSize(.mini).frame(width: 14)
        case .done: Image(systemName: "checkmark").foregroundStyle(.secondary).frame(width: 14)
        case .failed: Image(systemName: "xmark").foregroundStyle(Tint.failure).frame(width: 14)
        case .ended: Image(systemName: "stop").foregroundStyle(.secondary).frame(width: 14)
        }
    }
}

private struct ToolRow: View {
    let tool: AgentRow.Tool
    @State private var open = false

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                StatusMark(status: tool.status)
                if !tool.diff.isEmpty {
                    DisclosureButton(expanded: open, accessibilityLabel: "Diff", width: 12) { open.toggle() }
                }
                Text(tool.name).fontWeight(.medium)
                Text(tool.summary)
                    .font(.callout.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: Spacing.group)
                RunTimeChip(startedMs: tool.startedMs, endedMs: tool.endedMs)
            }
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
        VStack(alignment: .leading, spacing: 0) {
            ForEach(hunks.indices, id: \.self) { h in
                ForEach(hunks[h].lines.indices, id: \.self) { l in
                    let line = hunks[h].lines[l]
                    Text(line.isEmpty ? " " : line)
                        .font(.caption.monospaced())
                        .foregroundStyle(line.hasPrefix("-") ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .textSelection(.enabled)
        .padding(Spacing.group)
        .surface(.inset, in: .control)
    }
}

private struct SubagentRow: View {
    let subagent: AgentRow.Subagent

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                StatusMark(status: subagent.status)
                Text(NativeCopy.agentType(subagent.agentType)).fontWeight(.medium)
                Text(subagent.description).foregroundStyle(.secondary).lineLimit(1)
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
                    .padding(.leading, 22)
            }
        }
        .font(.callout)
        .padding(Spacing.group)
        .surface(.inset, in: .card)
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
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Text(state == "Withdrawn" ? "Withdrawn" : state == "Sent" ? "Sent from the queue" : "Queued")
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
            Text(text).lineLimit(2)
            if state == "Waiting", let sendNow {
                Button("Send Now", action: sendNow)
                    .controlSize(.small)
                    .help("Claude reads the queued messages now instead of after this turn.")
                    .identified("native-send-now")
            }
        }
        .padding(.horizontal, Spacing.inset)
        .padding(.vertical, Spacing.group)
        .surface(.inset, in: .card)
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
