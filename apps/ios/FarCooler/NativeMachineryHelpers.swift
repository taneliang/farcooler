import SwiftUI

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
        .foregroundStyle(.quietInk)
        .fixedSize()
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
        .foregroundStyle(.quietInk)
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
                case .done: Image(systemName: "checkmark").foregroundStyle(.quietInk)
                case .failed: Image(systemName: "xmark").foregroundStyle(Tint.failure)
                case .ended: Image(systemName: "stop").foregroundStyle(.quietInk)
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
                            .foregroundStyle(line.hasPrefix("-") ? AnyShapeStyle(Color.quietInk) : AnyShapeStyle(.primary))
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

