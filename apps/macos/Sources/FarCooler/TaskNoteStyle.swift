import AgentKit
import SwiftUI

/// How one kind of note in a task's record is drawn: its label, its icon and
/// how loud it is. Values, so `TaskNoteStyleTests` pins them.
///
/// One SF Symbols family (outlined, bubble and mark shapes), and system
/// semantic colors only. Color is only for what needs the person (ov-98,
/// owner): every kind's label is secondary, and the one accent in the record
/// is a question still waiting on its answer (`tint(of:in:status:)`).
struct TaskNoteStyle: Equatable {
    /// The semantic color a kind is tinted with.
    enum Tint: Equatable {
        case accent, secondary

        var color: Color {
            switch self {
            case .accent: return .accentColor
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
            return TaskNoteStyle(label: "Decision", symbol: "checkmark.seal", tint: .secondary, weight: .prominent)
        case .finding:
            return TaskNoteStyle(label: "Finding", symbol: "lightbulb", tint: .secondary, weight: .standard)
        case .progress:
            return TaskNoteStyle(label: "Progress", symbol: "chart.bar", tint: .secondary, weight: .standard)
        case .question:
            return TaskNoteStyle(label: "Question", symbol: "questionmark.bubble", tint: .secondary, weight: .standard)
        case .answer:
            return TaskNoteStyle(label: "Answer", symbol: "text.bubble", tint: .secondary, weight: .standard)
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

    /// A decision as drawn: its `extra.rejected` options when it has them,
    /// else the body split at "Rejected:", which is only for older notes
    /// written before the options were kept on the wire.
    static func decision(_ note: TaskNoteRow) -> Decision {
        let options = note.rejected.filter { !$0.isEmpty }
        guard options.isEmpty else {
            return Decision(chosen: note.body, rejected: options.joined(separator: "; "))
        }
        return decision(note.body)
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

    /// The color `note`'s label is drawn in, among `notes` (display order,
    /// `feed`): the accent for a question with no answer under it, in a task
    /// waiting on a decision, which is the one thing in the record that
    /// needs the person; its kind's own tint (secondary) for every other.
    static func tint(of note: TaskNoteRow, in notes: [TaskNoteRow], status: TaskStatus) -> Tint {
        guard note.kind == .question, status == .needsDecision,
            let at = notes.firstIndex(where: { $0.id == note.id })
        else { return of(note.kind).tint }
        let answered = notes.index(after: at) < notes.endIndex && notes[notes.index(after: at)].kind == .answer
        return answered ? of(note.kind).tint : .accent
    }

    /// The record as the task view draws it: newest first (`TaskNoteFeed`),
    /// with the answers that sit under their question.
    static func feed(_ notes: [TaskNoteRow]) -> (notes: [TaskNoteRow], paired: Set<String>) {
        let ordered = TaskNoteFeed.newestFirst(notes)
        return (ordered, answersPaired(ordered))
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
