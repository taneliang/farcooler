import Foundation

/// The order a task's record is drawn in, for every client: newest first, so
/// the latest word is at the top and nothing needs scrolling to.
///
/// A question and the answer written straight after it are one pair, which is
/// ordered by its newest member (the answer's time, once answered) and reads
/// question above answer inside. The "Created" entry is the task's first and
/// sits at the bottom.
public enum TaskNoteFeed {
    /// `notes` (oldest first, as the store sends them) in display order.
    public static func newestFirst(_ notes: [TaskNoteRow]) -> [TaskNoteRow] {
        var groups: [[TaskNoteRow]] = []
        var created: [TaskNoteRow] = []
        for note in notes {
            if note.kind == .created {
                created.append(note)
            } else if note.kind == .answer, let last = groups.last, last.count == 1,
                last[0].kind == .question
            {
                groups[groups.count - 1].append(note)
            } else {
                groups.append([note])
            }
        }
        // Stable sort by newest member; equal times keep the later group above.
        let ordered = groups.enumerated().sorted { lhs, rhs in
            let l = lhs.element.last!.at
            let r = rhs.element.last!.at
            return l != r ? l > r : lhs.offset > rhs.offset
        }
        return ordered.flatMap(\.element) + created.reversed()
    }

    /// The ids of answers that directly follow a question in `notes` (the
    /// display order), which are drawn as its reply.
    public static func answersPaired(_ notes: [TaskNoteRow]) -> Set<String> {
        var paired = Set<String>()
        for (previous, note) in zip(notes, notes.dropFirst())
        where previous.kind == .question && note.kind == .answer {
            paired.insert(note.id)
        }
        return paired
    }
}
