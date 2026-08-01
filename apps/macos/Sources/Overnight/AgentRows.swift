import AgentKit
import SwiftUI

/// One row of a rendered agent transcript.
///
/// A thin switch, deliberately: `Transcript` already decided what happened
/// (coalesced chunks, mutated a tool in place, kept a gap as its own row) —
/// this only decides how each of the three shapes it can hand back gets drawn.
struct AgentRowView: View {
    let row: TranscriptRow

    var body: some View {
        switch row {
        case let .message(role, text):
            MessageRow(role: role, text: text)
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
                Text(text)
                    .font(.body)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 32)
            }

        case .thought:
            // Collapsed by default: a thought is the agent's scratch work,
            // useful when something went wrong and noise the rest of the
            // time. `DisclosureGroup` with no binding starts closed, which is
            // exactly the state most sessions never need to leave.
            DisclosureGroup {
                Text(text)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .padding(.top, 2)
            } label: {
                Text("Thought")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// One tool call, mutated in place by every update — never a new row — so a
/// call that reports progress four times still occupies the one line it
/// earned.
private struct ToolRowView: View {
    let tool: ToolRow

    private var expandable: Bool { tool.content != nil || tool.diff != nil }

    var body: some View {
        if expandable {
            DisclosureGroup {
                VStack(alignment: .leading, spacing: 8) {
                    if let content = tool.content, !content.isEmpty {
                        Text(content)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                    if let diff = tool.diff {
                        DiffView(diff: diff)
                    }
                }
                .padding(.top, 4)
            } label: {
                label
            }
        } else {
            label
        }
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
