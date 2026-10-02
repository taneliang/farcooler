import AgentKit
import SwiftUI

/// How one kind of note in a task's record is drawn: its label, its icon and
/// how loud it is. Values, so `TaskNoteStyleTests` pins them.
///
/// One SF Symbols family (outlined, bubble and mark shapes), and system
/// semantic colors only, so each reads in light and dark mode without a
/// palette of its own. Color is spent where it means something: a decision is
/// the accent, a question that waits is orange, and an answer, which closes
/// one, is green. Everything else is quiet.
struct TaskNoteStyle: Equatable {
    /// The semantic color a kind is tinted with.
    enum Tint: Equatable {
        case accent, orange, green, primary, secondary

        var color: Color {
            switch self {
            case .accent: return .accentColor
            case .orange: return .orange
            case .green: return .green
            case .primary: return .primary
            case .secondary: return .secondary
            }
        }
    }

    /// How much of the page a note takes.
    enum Weight: Equatable {
        /// The decision: a card, with its text a step heavier.
        case prominent
        /// Written by somebody: label, icon and body at the normal size.
        case standard
        /// The store's own history: one muted line.
        case quiet
    }

    var label: String
    var symbol: String
    var tint: Tint
    var weight: Weight

    static func of(_ kind: TaskNoteKind) -> TaskNoteStyle {
        switch kind {
        case .decision:
            return TaskNoteStyle(label: "Decision", symbol: "checkmark.seal", tint: .accent, weight: .prominent)
        case .finding:
            return TaskNoteStyle(label: "Finding", symbol: "lightbulb", tint: .primary, weight: .standard)
        case .progress:
            return TaskNoteStyle(label: "Progress", symbol: "chart.bar", tint: .secondary, weight: .standard)
        case .question:
            return TaskNoteStyle(label: "Question", symbol: "questionmark.bubble", tint: .orange, weight: .standard)
        case .answer:
            return TaskNoteStyle(label: "Answer", symbol: "text.bubble", tint: .green, weight: .standard)
        case .comment:
            return TaskNoteStyle(label: "Comment", symbol: "bubble.left", tint: .secondary, weight: .standard)
        case .statusChange:
            return TaskNoteStyle(label: "Status Change", symbol: "arrow.right.circle", tint: .secondary, weight: .quiet)
        case .created:
            return TaskNoteStyle(label: "Created", symbol: "plus.circle", tint: .secondary, weight: .quiet)
        }
    }

    /// A decision's body, split at its "Rejected:" clause, which is where
    /// the manager skill's `--rejected` options land in the prose.
    struct Decision: Equatable {
        var chosen: String
        var rejected: String?
    }

    static func decision(_ body: String) -> Decision {
        guard let range = body.range(of: "Rejected:", options: .caseInsensitive) else {
            return Decision(chosen: body, rejected: nil)
        }
        let chosen = body[..<range.lowerBound].trimmingCharacters(in: .whitespacesAndNewlines)
        let rejected = body[range.upperBound...].trimmingCharacters(in: .whitespacesAndNewlines)
        guard !chosen.isEmpty, !rejected.isEmpty else { return Decision(chosen: body, rejected: nil) }
        return Decision(chosen: chosen, rejected: rejected)
    }

    /// The ids of answers that directly follow a question, which are drawn
    /// as its reply: indented under it, on a rule.
    static func answersPaired(_ notes: [TaskNoteRow]) -> Set<String> {
        var paired = Set<String>()
        for (previous, note) in zip(notes, notes.dropFirst())
        where previous.kind == .question && note.kind == .answer {
            paired.insert(note.id)
        }
        return paired
    }
}

extension TaskColumnModel {
    /// The least height the task's text keeps above the divider, so the
    /// record is never squeezed to a line or two.
    static let minimumContentHeight: CGFloat = 260

    /// The top's height in a split of `total`, from its `share`: never under
    /// `minimum` (when the view has the room), never into the bottom's
    /// `minimumShare`.
    static func topHeight(total: CGFloat, share: Double, minimum: CGFloat) -> CGFloat {
        let ceiling = total * (1 - minimumShare)
        let floor = min(minimum, ceiling)
        return min(max(total * share, floor), ceiling)
    }
}
