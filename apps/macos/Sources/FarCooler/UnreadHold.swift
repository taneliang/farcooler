import AgentKit
import Foundation

/// A task selected, and what had been read when it was (ov-177).
///
/// Opening a task reads it (`BoardReads.open`), which used to take its lines
/// out of Unread at once: the line clicked vanished from under the pointer
/// and the selection jumped to the task's row in its status. Now Unread
/// lists the selected task by the reads as they were when it was selected,
/// so its lines stay where they are while it's selected, opened or Mark All
/// as Read or not, and leave, on the shared spring, once the selection moves
/// to another task or is cleared. The Done rule keeps a read row in its
/// section the same way (`keeping`, ov-104): what's selected stays where it
/// is.
struct HeldRead: Equatable {
    let taskID: String
    /// The reads when it was selected.
    let reads: BoardReads
}

extension BoardSummary {
    /// What's unread on `rows` by `reads`, but for `held`'s task, which is
    /// listed by the reads it was selected under.
    static func make(
        rows: [TaskRow], notes: [String: [TaskNoteRow]], reads: BoardReads, held: HeldRead?
    ) -> BoardSummary {
        guard let held, let kept = rows.first(where: { $0.id == held.taskID }) else {
            return make(rows: rows, notes: notes, reads: reads)
        }
        let others = make(rows: rows.filter { $0.id != held.taskID }, notes: notes, reads: reads)
        let own = make(rows: [kept], notes: notes, reads: held.reads)
        func newest(_ items: [Item]) -> [Item] { items.sorted { $0.at > $1.at } }
        return BoardSummary(
            finished: newest(others.finished + own.finished), moved: newest(others.moved + own.moved),
            created: newest(others.created + own.created),
            activity: (others.activity + own.activity).sorted { $0.at > $1.at })
    }
}

extension TaskBoardStore {
    /// The tasks whose records Unread reads for notes: those that moved since
    /// they were read, and the selected one by the reads it was selected
    /// under, so its Activity line stays too.
    func noteCandidates(reads: BoardReads) -> [TaskRow] {
        var picked = BoardSummary.noteCandidates(rows: board.rows, reads: reads)
        if let held, !picked.contains(where: { $0.id == held.taskID }),
            let row = board.rows.first(where: { $0.id == held.taskID }),
            !BoardSummary.noteCandidates(rows: [row], reads: held.reads).isEmpty
        {
            picked.append(row)
        }
        return picked
    }
}
