import AgentKit
import SwiftUI

struct AgentRowView: View {
    let row: TranscriptRow
    /// Whether this is the newest row, which is what makes a thought "live".
    ///
    /// A thought is still being written exactly while nothing has followed it.
    /// That needs no extra state in the model: the transcript already knows
    /// the order, and asking "is anything after me" is the same question.
    var isLast: Bool = false

    var body: some View {
        switch row.kind {
        case let .message(role, text):
            MessageRow(role: role, text: text, isLive: isLast)
        case let .tool(tool):
            ToolRowView(tool: tool)
        case let .gap(reason):
            GapRow(reason: reason)
        }
    }
}

/// One message. Three shapes for three roles, because they answer three
/// different questions: what did I say, what did it say, and what did it
/// think before saying that.
private struct MessageRow: View {
    let role: Role
    let text: String
    var isLive: Bool = false

    var body: some View {
        switch role {
        case .user:
            // Right-aligned with a fill, the one place in the transcript that
            // is not the agent talking — it has to read as a different voice
            // at a glance, not just on close reading.
            HStack {
                Spacer(minLength: 32)
                Text(text)
                    .font(.body)
                    .textSelection(.enabled)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    // `quaternary` rather than a hand-mixed opacity: it is the
                    // fill AppKit uses for exactly this — a grouped surface
                    // that must stay legible in both appearances without
                    // anyone picking two numbers and hoping.
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 14))
            }

        case .agent:
            // Plain body text, full width. This is the common case and the
            // one that should cost the eye nothing extra to read.
            HStack {
                MarkdownText(text: text)
                Spacer(minLength: 32)
            }

        case .thought:
            // Open while it is being written, closed once it is done.
            //
            // Watching an agent think is the interesting part, and a
            // permanently collapsed row hides the only thing happening during
            // a long turn. But a finished thought is scratch work, and a
            // transcript of them is unreadable — so it folds itself away the
            // moment anything follows it.
            ThoughtRow(text: text, isLive: isLive)
        }
    }
}

/// The agent's reasoning: streaming while it happens, folded away after.
private struct ThoughtRow: View {
    let text: String
    let isLive: Bool

    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(Motion.snap) { expanded.toggle() }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .rotationEffect(.degrees(showing ? 90 : 0))
                    Text(isLive ? "Thinking…" : "Thought")
                        .font(.caption.weight(.medium))
                }
                .foregroundStyle(.secondary)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if showing {
                // While it is being written, only the last few lines — enough
                // to see it moving, which is the whole point. A thinking agent
                // that shows one collapsed word looks stuck, and one that
                // shows everything pushes the conversation off screen.
                MarkdownText(text: isLive && !expanded ? Self.tail(of: text) : text, secondary: true)
                    .padding(.leading, 2)
            }
        }
        // Driven by the model, not by a timer: a thought stops being live the
        // instant something follows it, and the fold should follow that.
        .animation(Motion.snap, value: isLive)
        .animation(Motion.snap, value: text)
    }

    /// Open while live unless the reader has closed it; closed after unless
    /// the reader has opened it.
    private var showing: Bool { expanded || isLive }

    /// The last few lines, for a thought still being written.
    private static func tail(of text: String, lines: Int = 5) -> String {
        let all = text.split(separator: "\n", omittingEmptySubsequences: false)
        guard all.count > lines else { return text }
        return all.suffix(lines).joined(separator: "\n")
    }
}

/// A bounded, scrollable block of output.
///
/// Bounded because a tool can return a thousand lines and a transcript is not
/// a place to page through them; scrollable because truncating to a preview
/// throws away the half you needed. The same box serves reasoning, console
/// output and file contents, so they are not three inventions.
struct DetailBox: View {
    let text: String
    var monospaced: Bool = true

    var body: some View {
        ScrollView {
            Text(text)
                .font(monospaced ? .caption.monospaced() : .caption)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(8)
        }
        .frame(maxHeight: 220)
        .background(.quinary, in: RoundedRectangle(cornerRadius: 7))
    }
}

/// One tool call, mutated in place by every update — never a new row — so a
/// call that reports progress four times still occupies the one line it
/// earned.
private struct ToolRowView: View {
    let tool: ToolRow

    private var expandable: Bool { tool.content != nil || tool.diff != nil }

    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if expandable {
                Button {
                    withAnimation(Motion.snap) { expanded.toggle() }
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "chevron.right")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .rotationEffect(.degrees(expanded ? 90 : 0))
                        label
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            } else {
                label
            }

            if expanded {
                VStack(alignment: .leading, spacing: 8) {
                    if let content = tool.content, !content.isEmpty {
                        DetailBox(text: content)
                    }
                    if let diff = tool.diff {
                        DiffView(diff: diff)
                    }
                }
            }
        }
        .animation(Motion.snap, value: expanded)
    }

    private var label: some View {
        HStack(spacing: 7) {
            // The same dot the sidebar uses for a terminal's status, mapped
            // onto a tool call's own four states — one vocabulary for
            // "something is happening" everywhere it appears, rather than a
            // second one invented for this row.
            StatusGlyph(status: status, size: 7)
            Text(tool.title)
                .font(.callout.weight(.medium))
                // One line, always. An adapter puts the whole absolute path in
                // the title, which wrapped to six lines in a narrow pane and
                // turned a one-line status row into the largest thing on
                // screen.
                .lineLimit(1)
                .truncationMode(.middle)
            if let location = tool.locations.first {
                Text(location)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 4)
        }
        // Inset on a faint fill so a tool call reads as machinery rather than
        // as something the agent said. Without it the transcript is one
        // undifferentiated column of text and the eye cannot find the prose.
        .padding(.horizontal, 9)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quinary, in: RoundedRectangle(cornerRadius: 7))
    }

    private var status: Status {
        switch tool.status {
        case .pending: return .starting
        case .inProgress: return .working
        case .completed: return .done
        case .failed: return .failed
        }
    }
}

/// A break in the transcript, named rather than hidden.
///
/// This is the one row in the whole feature that is not allowed to be quiet.
/// `StatusGlyph`'s silence-by-default rule is for a terminal that has nothing
/// to say; a gap is the opposite of nothing — it is the transcript admitting
/// history is missing — and rendering it as a thin rule between two messages
/// would let a reader miss the one fact this design exists to never hide.
private struct GapRow: View {
    let reason: GapReason

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "scissors")
                .font(.system(size: 11))
            Text(sentence)
                .font(.system(size: 11.5, weight: .medium))
            Spacer(minLength: 0)
        }
        .foregroundStyle(.orange)
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.orange.opacity(0.10), in: RoundedRectangle(cornerRadius: 6))
    }

    private var sentence: String {
        switch reason {
        case .ringTrimmed:
            return "Some earlier history was trimmed and is not shown here."
        case .loadUnsupported:
            return "This session could not be loaded from where it left off."
        case .unparsed:
            return "Something happened here that this version cannot show."
        }
    }
}
