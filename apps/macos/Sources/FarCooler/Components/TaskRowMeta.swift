import AgentKit
import SwiftUI

/// A task row's one metadata line (ov-92, owner, 2 Oct: "the blue checkmark
/// is very distracting"): "<status or time> · <progress>", e.g. "Done 6d ago"
/// or "Updated 2h ago · 3 of 5", quiet, with a monochrome checklist glyph
/// before the progress. Color only where the row needs the person: a
/// decision waiting or a task blocked on another. A task that hasn't moved
/// in a day says so in the same quiet text, "In Progress · no movement for
/// 3d", with no color, icon or border of its own. Labels live in the task
/// view, not here.
enum TaskRowMeta {
    enum Tone: Equatable {
        /// Secondary, the row's ordinary metadata.
        case quiet
        /// The accent: the row needs the person.
        case attention
    }

    struct Line: Equatable {
        /// What it says first: why it's blocked, its staleness, or its time.
        var lead: String?
        /// Its acceptance progress, "3 of 5", or nil.
        var progress: String?
        var tone: Tone

        var isEmpty: Bool { lead == nil && progress == nil }
        /// Read as one line, for VoiceOver and the tests.
        var text: String {
            [lead, progress.map { "\($0) met" }].compactMap { $0 }.joined(separator: " · ")
        }
    }

    /// Whether the row needs the person: the only rows with any color.
    static func needsAttention(_ row: TaskRow) -> Bool {
        row.status == .needsDecision || !row.blockedBy.isEmpty
    }

    /// The progress shown: "3 of 5" while a line is open, or on an active
    /// row with every line met; nothing on a Done or Canceled row with all
    /// of them met, which is what's expected, nor with no lines at all.
    static func progress(_ row: TaskRow) -> String? {
        guard let progress = row.acceptanceProgress, progress.total > 0 else { return nil }
        if progress.isComplete, row.status.isFinished { return nil }
        return "\(progress.met) of \(progress.total)"
    }

    static func line(_ row: TaskRow, at now: Date) -> Line {
        let lead = row.blockedSummary ?? stale(row, at: now) ?? row.timeNote(at: now)
        return Line(lead: lead, progress: progress(row), tone: needsAttention(row) ? .attention : .quiet)
    }

    /// "In Progress · no movement for 3d", or nil for a row still moving.
    static func stale(_ row: TaskRow, at now: Date) -> String? {
        guard row.staleness(at: now) == .stale else { return nil }
        let days = max(1, Int(row.stoppedFor(at: now) / 86_400))
        return "\(row.status.title) · no movement for \(days)d"
    }
}

/// The metadata line, drawn: one size, one quiet color unless the row needs
/// the person, and the checklist glyph monochrome beside the progress.
struct TaskRowMetaView: View {
    let row: TaskRow

    static func color(_ tone: TaskRowMeta.Tone) -> Color {
        switch tone {
        case .quiet: .secondary
        case .attention: .accentColor
        }
    }

    var body: some View {
        BoardTick { now in
            let line = TaskRowMeta.line(row, at: now)
            if !line.isEmpty {
                HStack(spacing: 4) {
                    if let lead = line.lead { Text(lead).lineLimit(1) }
                    if line.lead != nil, line.progress != nil { Text("·") }
                    if let progress = line.progress {
                        Image(systemName: "checklist")
                        Text(progress).monospacedDigit()
                    }
                }
                .font(.system(size: WorkspaceStyle.PaneText.minimum))
                .foregroundStyle(Self.color(line.tone))
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(line.text)
            }
        }
    }
}
