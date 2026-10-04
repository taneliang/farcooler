import SwiftUI

// Unread (ov-104, ov-113), on a phone: the first section of a workspace's
// board, listing what's new to this person: what finished, what moved to Needs
// Decision or In Review, what was filed, and the newest note on each ticket
// that has any. It's the Mac's section (`BoardSummary`, AgentKit's), so the
// two say the same thing about the same board.
//
// A line stays until its ticket is opened (`BoardReads`), and opening a ticket
// on any of this person's devices clears it on all of them, when the runner
// keeps the state. Mark All as Read asks first, and says that.

struct BoardUnreadSection: View {
    let board: TaskBoardModel
    let reads: BoardReads
    /// A ticket's notes, read from the runner (`task.get`); nil when that
    /// didn't come back.
    let readNotes: (TaskRow) async -> [TaskNoteRow]?
    /// Whether Mark All as Read reaches every device.
    let readsAreShared: () -> Bool
    let onOpen: (TaskRow) -> Void
    /// Everything on the board is read, through the notes read for Unread.
    let onMarkAllRead: (_ latest: Date?) -> Void

    /// The notes read, by task, each kept until its task's `updatedAt` moves.
    @State private var cache: [String: (updatedAt: Date?, notes: [TaskNoteRow])] = [:]
    @State private var notes: [String: [TaskNoteRow]] = [:]
    @State private var asking = false

    /// The most tickets whose notes a phone reads for Unread: each is a call.
    static let noteLimit = 10

    private var summary: BoardSummary {
        BoardSummary.make(rows: board.rows, notes: notes, reads: reads)
    }

    var body: some View {
        let summary = summary
        Section {
            if summary.isEmpty {
                Text(BoardSummary.nothing)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("board-unread-empty")
            } else {
                group("Finished", summary.finished)
                group("Needs You or Review", summary.moved)
                group("New", summary.created)
                activity(summary.activity)
            }
        } header: {
            header(summary)
        }
        .task(id: notesKey) { await readNotesForUnread() }
    }

    private func header(_ summary: BoardSummary) -> some View {
        HStack(spacing: PaneMetrics.tight) {
            Text("Unread")
            if !summary.isEmpty {
                Text("\(summary.count)")
                    .font(.subheadline.monospacedDigit())
                    .foregroundStyle(.tertiary)
            }
            Spacer()
            if !summary.isEmpty {
                Button("Mark All as Read") { asking = true }
                    .font(.footnote)
                    .textCase(nil)
                    .accessibilityIdentifier("board-mark-all-read")
                    .alert("Mark All as Read?", isPresented: $asking) {
                        Button("Cancel", role: .cancel) {}
                        Button("Mark as Read") { onMarkAllRead(newestNote) }
                    } message: {
                        Text(
                            BoardSummary.markAllReadMessage(
                                tasks: summary.taskCount, everywhere: readsAreShared()))
                    }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-unread")
    }

    @ViewBuilder
    private func group(_ title: String, _ items: [BoardSummary.Item]) -> some View {
        if !items.isEmpty {
            let capped = BoardSummary.capped(items)
            groupTitle(title, count: items.count)
            ForEach(capped.shown) { item in
                line(taskID: item.taskID, key: item.key, title: item.title, id: "board-unread-\(item.id)") {
                    BoardTick { now in
                        Text(item.when(now: now)).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            more(capped.more)
        }
    }

    @ViewBuilder
    private func activity(_ entries: [BoardSummary.Activity]) -> some View {
        if !entries.isEmpty {
            let capped = BoardSummary.capped(entries)
            groupTitle("Activity", count: entries.count)
            ForEach(capped.shown) { entry in
                line(taskID: entry.taskID, key: entry.key, title: entry.title, id: "board-unread-activity-\(entry.key)") {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(Text(entry.kind.title).foregroundStyle(.secondary))  \(entry.text)")
                            .font(.caption)
                            .lineLimit(2)
                        BoardTick { now in
                            Text(entry.foot(now: now)).font(.caption2).foregroundStyle(.tertiary)
                        }
                    }
                }
            }
            more(capped.more)
        }
    }

    private func groupTitle(_ title: String, count: Int) -> some View {
        HStack {
            Text(title)
            Spacer()
            Text("\(count)").monospacedDigit().foregroundStyle(.tertiary)
        }
        .font(.footnote.weight(.semibold))
        .foregroundStyle(.secondary)
        .accessibilityElement(children: .combine)
    }

    private func line<Detail: View>(
        taskID: String, key: String, title: String, id: String, @ViewBuilder detail: () -> Detail
    ) -> some View {
        Button {
            if let row = board.rows.first(where: { $0.id == taskID }) { onOpen(row) }
        } label: {
            VStack(alignment: .leading, spacing: PaneMetrics.tight) {
                Text(key)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.secondary)
                Text(title).font(.subheadline).lineLimit(2)
                detail()
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityHint("Opens the task")
        .accessibilityIdentifier(id)
    }

    @ViewBuilder
    private func more(_ count: Int) -> some View {
        if count > 0 {
            Text("and \(count) more").font(.footnote).foregroundStyle(.secondary)
        }
    }

    // MARK: - Notes

    /// The newest note read for Unread: Mark All as Read goes through it, so
    /// a note written after it stays unread.
    private var newestNote: Date? { notes.values.flatMap { $0 }.map(\.at).max() }

    /// What a notes read is keyed on: the tickets worth reading and where
    /// each stands, and what's read, so a ticket opened reads again.
    private struct NotesKey: Hashable {
        var tasks: [String]
        var reads: String
    }

    private var notesKey: NotesKey {
        let marks = reads.opened.values.map(\.timeIntervalSince1970).reduce(0, +)
        return NotesKey(
            tasks: candidates.map { "\($0.id)@\($0.updatedAt?.timeIntervalSince1970 ?? 0)" },
            reads: "\(reads.floor.timeIntervalSince1970) \(marks)")
    }

    private var candidates: [TaskRow] {
        BoardSummary.noteCandidates(rows: board.rows, reads: reads, limit: Self.noteLimit)
    }

    /// Read the records of the tickets that moved since they were read, so
    /// Unread can list their notes: each remembered until its `updatedAt` moves.
    private func readNotesForUnread() async {
        let picked = candidates
        for row in picked where cache[row.id]?.updatedAt != row.updatedAt {
            guard let read = await readNotes(row) else { continue }
            cache[row.id] = (row.updatedAt, read)
        }
        notes = Dictionary(
            uniqueKeysWithValues: picked.compactMap { row in cache[row.id].map { (row.id, $0.notes) } })
    }
}
