import AgentKit
import SwiftUI

// A repository's board.
//
// Everything with a rule in it lives in `TaskBoardModel` in AgentKit, which is
// what `swift test --package-path apps/shared/AgentKit` runs on every push.
// This file draws it and nothing else: no sentence is composed here, no
// staleness is decided here, and no status word is spelled here.
//
// ## The one thing this surface must never grow
//
// Current understanding is mutable and lives on the task row; the record of
// how you got there is append-only and lives in typed notes that can be
// superseded but never edited. The board is where somebody could quietly undo
// that, and the store would not stop it at review — `task_notes` has a
// `BEFORE UPDATE` trigger that refuses unconditionally, so an "Edit Note…"
// here would compile, ship, and fail at runtime in front of a user.
//
// So the record below is drawn from `TaskDetailModel` and offers nothing at
// all, and every write this board makes goes through `BoardAction`, whose
// `rewritesTheRecord` is asserted over in `TaskBoardModelTests`. A new write
// belongs in that list, not in a `Button` written by hand here.

/// One repository's board, as this window last read it.
///
/// One store per repository per client, held by `ContentView` — see
/// `boardStore(for:client:)` there, which follows `changesStore(for:client:)`
/// exactly: a `DaemonClient` is dropped when its runner leaves, and a store
/// held over from the old one would go on talking to a connection nobody is
/// answering.
@MainActor
final class TaskBoardStore: ObservableObject {
    @Published private(set) var board: TaskBoardModel = .empty
    @Published private(set) var detail: TaskDetailModel = .empty
    /// The card that is open, or nil for the board alone.
    @Published var opened: TaskRow?
    /// Whether a read has ever come back.
    ///
    /// Separate from "the board is empty", which is a real and different
    /// answer: a repository with no tasks on it should say so, and one that
    /// has not been read yet must not.
    @Published private(set) var hasRead = false
    @Published private(set) var reading = false
    /// What to say when a read or a move did not work.
    ///
    /// This app's sentence, never the runner's. The CLI's own stderr is
    /// written for whoever is reading a terminal — it names methods and Rust
    /// types — and putting it on a board would be the one thing the refusal
    /// vocabulary exists to prevent. It is dropped rather than tucked into a
    /// tooltip, because a tooltip is still a screen.
    @Published private(set) var trouble: String?

    let repository: Repository
    /// Held, and readable, so the window can tell a store built against a
    /// dropped connection from one built against the live link — see
    /// `boardStore(for:client:)` in `ContentView`, which is the same identity
    /// check `changesStore(for:client:)` makes for the same reason.
    let client: DaemonClient
    /// The `boardGeneration` this store has already acted on, so an event that
    /// arrives while a read is in flight is not read twice.
    private var seenGeneration = 0

    init(client: DaemonClient, repository: Repository) {
        self.client = client
        self.repository = repository
        self.seenGeneration = client.boardGeneration
    }

    /// Re-read the whole board.
    ///
    /// Whole, and not a delta, because the daemon's event carries none — three
    /// editors move this state at once, and a client applying deltas would
    /// need to be right about all three. `task list` answers a board in one
    /// call, which is what makes re-reading the cheap answer.
    func reload() async {
        reading = true
        defer { reading = false }
        let (data, _) = await client.taskBoard(repository: repository.id)
        guard let data else {
            trouble = "Far Cooler couldn’t read this board."
            return
        }
        guard let read = try? TaskBoardModel.decode(data) else {
            trouble = "This runner answered with a board this version can’t read."
            return
        }
        trouble = nil
        board = read
        hasRead = true
        // Keep the open card pointing at the row that is on the board now,
        // rather than at the copy this store read a minute ago — the whole
        // reason the board re-reads is that the row may have moved.
        if let opened, let fresh = read.rows.first(where: { $0.id == opened.id }) {
            self.opened = fresh.with(blocks: opened.blockedBy)
        }
    }

    /// Re-read only if a runner has said something moved since the last read.
    ///
    /// The board deliberately re-reads for EVERY actor, including `user`. The
    /// actor is on the event so a client can skip its own writes, and this one
    /// does not take that shortcut: a click in this window and a `farcooler
    /// task set` typed in a terminal beside it are both `user`, and a board
    /// that dropped `user` would go blind to the second — which is the one it
    /// could not have predicted.
    func reloadIfMoved() async {
        guard client.boardGeneration != seenGeneration else { return }
        seenGeneration = client.boardGeneration
        await reload()
        if let opened { await open(opened) }
    }

    /// Open one card: its record, and what it is waiting on.
    ///
    /// A second call, because `task list` carries neither — one call for a
    /// whole board is what makes surveying it cheap, and the record is the
    /// expensive half.
    func open(_ row: TaskRow) async {
        opened = row
        detail = .empty
        let (data, _) = await client.taskDetail(key: row.key, repository: repository.id)
        guard let data, let read = try? TaskDetailModel.decode(data) else {
            trouble = "Far Cooler couldn’t read this task."
            return
        }
        trouble = nil
        detail = read
        // The blocks arrive as ids; the board is what turns them into keys.
        opened = row.with(blocks: board.resolvingBlocks(read.blocks))
    }

    /// Move a task to another column.
    ///
    /// Re-reads on the way back rather than moving the row locally. The runner
    /// writes the move and its `status_change` note in one transaction, and a
    /// board that moved the card itself would be showing a state it decided
    /// rather than one the record holds.
    func move(_ row: TaskRow, to status: TaskStatus) async {
        if let refused = await client.moveTask(
            key: row.key, to: status.rawValue, repository: repository.id)
        {
            // The runner's own words are not drawn — see `trouble`. What is
            // worth saying is which task did not move, because a board full of
            // cards makes "it didn't work" useless.
            _ = refused
            trouble = "\(row.key) didn’t move."
            return
        }
        await reload()
    }
}

extension TaskRow {
    /// This row with its blocks filled in from a detail read.
    fileprivate func with(blocks: [TaskBlockRef]) -> TaskRow {
        var copy = self
        copy.blockedBy = blocks
        return copy
    }
}

/// The board itself.
struct TaskBoardSheet: View {
    @ObservedObject var store: TaskBoardStore
    @ObservedObject var client: DaemonClient
    let onClose: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            if !store.hasRead && store.reading {
                centered { ProgressView() }
            } else if let trouble = store.trouble, !store.hasRead {
                centered {
                    VStack(spacing: 10) {
                        Text(trouble)
                        Button("Try Again") { Task { await store.reload() } }
                    }
                }
            } else if store.hasRead && store.board.rows.isEmpty
                && store.board.unreadable.isEmpty
            {
                centered {
                    VStack(spacing: 6) {
                        Text("Nothing on this board yet.").font(.headline)
                        Text("Put a task on it with farcooler task create.")
                            .foregroundStyle(.secondary)
                    }
                }
            } else {
                columns
            }
        }
        .frame(minWidth: 900, minHeight: 520)
        .background(WorkspaceStyle.canvas)
        // The board only moves when a runner says it did. Polling a board that
        // nothing is touching would be the shape this app spent a release
        // removing.
        .task(id: client.boardGeneration) { await store.reloadIfMoved() }
        .task { if !store.hasRead { await store.reload() } }
        .sheet(item: $store.opened) { row in
            TaskCard(row: row, detail: store.detail, onClose: { store.opened = nil })
        }
    }

    private func centered<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack { Spacer(); content(); Spacer() }.frame(maxWidth: .infinity)
    }

    private var header: some View {
        HStack(spacing: 10) {
            Text(store.repository.displayName).font(WorkspaceStyle.sectionTitle)
            // The one count worth putting in a title bar, and the sentence is
            // the model's like every other one here. Nothing when nothing is
            // waiting — `waitingSentence` is nil at zero, because a badge
            // reading zero teaches people to ignore it.
            if let waiting = store.board.waitingSentence {
                Text(waiting)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold))
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(Color.accentColor.opacity(0.18), in: Capsule())
            }
            Spacer()
            if store.reading { ProgressView().controlSize(.small) }
            // Shown beside the board rather than over it: a failed re-read
            // leaves the last good board on screen, and hiding it behind an
            // error would cost more than the error is worth.
            if let trouble = store.trouble, store.hasRead {
                Text(trouble)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
            }
            Button("Refresh") { Task { await store.reload() } }
            Button("Done", action: onClose).keyboardShortcut(.defaultAction)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(WorkspaceStyle.paneChrome)
    }

    private var columns: some View {
        ScrollView(.horizontal) {
            HStack(alignment: .top, spacing: 12) {
                ForEach(store.board.columns) { column in
                    TaskColumnView(column: column, store: store)
                }
                if !store.board.unreadable.isEmpty {
                    UnreadableColumnView(rows: store.board.unreadable)
                }
            }
            .padding(14)
        }
    }
}

/// One column, headed by its status.
private struct TaskColumnView: View {
    let column: TaskBoardColumn
    @ObservedObject var store: TaskBoardStore

    /// The only state waiting on the person looking at the board, so it is the
    /// only one drawn in the accent color. Everything competing for
    /// prominence is nothing having it.
    private var leads: Bool { column.status == .needsDecision }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Text(column.title)
                    .font(WorkspaceStyle.sectionTitle)
                    .foregroundStyle(leads ? Color.accentColor : Color.primary)
                Text("\(column.rows.count)")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
            }
            ScrollView {
                LazyVStack(spacing: 8) {
                    ForEach(column.rows) { row in
                        TaskCardRow(row: row, prominent: leads, store: store)
                    }
                }
            }
        }
        .frame(width: 260)
        .padding(10)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(leads ? Color.accentColor.opacity(0.07) : WorkspaceStyle.document))
    }
}

/// One card.
private struct TaskCardRow: View {
    let row: TaskRow
    let prominent: Bool
    @ObservedObject var store: TaskBoardStore

    private var stale: Bool { row.staleness == .stale }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(row.key)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                    .foregroundStyle(.secondary)
                Spacer()
                // A stale row is visibly different, which is the board's whole
                // job beyond showing state: a task sitting in `todo` that you
                // assumed was in flight is the failure mode of the factory,
                // and one rendered identically to a task that moved a minute
                // ago is what lets it happen.
                if stale {
                    Image(systemName: "clock.badge.exclamationmark")
                        .foregroundStyle(.orange)
                        .font(.system(size: WorkspaceStyle.PaneText.body))
                }
            }
            Text(row.title)
                .font(
                    .system(
                        size: WorkspaceStyle.PaneText.body,
                        weight: prominent ? .semibold : .regular)
                )
                .lineLimit(3)
                .fixedSize(horizontal: false, vertical: true)
            // Both sentences come from the model. Composing either here would
            // put the board's only real copy where nothing reads it back.
            if let call = row.callToAction {
                Text(call)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                    .foregroundStyle(Color.accentColor)
            }
            if let note = row.stalenessNote(at: Date()) {
                Text(note)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.orange)
            }
            if !row.labels.isEmpty {
                Text(row.labels.joined(separator: " · "))
                    .font(.system(size: WorkspaceStyle.PaneText.minimum))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 8).fill(WorkspaceStyle.paneChrome)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(
                    stale ? Color.orange.opacity(0.55) : WorkspaceStyle.hairline,
                    lineWidth: stale ? 1 : 0.5)
        )
        .contentShape(Rectangle())
        .onTapGesture { Task { await store.open(row) } }
        .contextMenu {
            // Built from the model's list rather than written out here, which
            // is what makes `nothingTheBoardOffersRewritesTheRecord` a guard
            // over what actually ships. A new write goes in `TaskBoardModel`,
            // beside the rule that checks it.
            Section("Move To") {
                ForEach(TaskBoardModel.moves(for: row)) { move in
                    Button(move.action.title) { Task { await store.move(row, to: move.status) } }
                }
            }
        }
    }
}

/// Rows this build has no column for.
///
/// A runner ahead of this app can name a status it has never heard of. Showing
/// it under a heading that says so is the only honest answer: dropping the row
/// makes work vanish from a board whose whole claim is that it shows the work.
private struct UnreadableColumnView: View {
    let rows: [UnreadableTaskRow]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Not On This Version").font(WorkspaceStyle.sectionTitle)
            Text("This runner uses states this Far Cooler doesn’t have yet.")
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            ForEach(rows) { row in
                VStack(alignment: .leading, spacing: 3) {
                    Text(row.key)
                        .font(
                            .system(
                                size: WorkspaceStyle.PaneText.secondary, design: .monospaced)
                        )
                        .foregroundStyle(.secondary)
                    Text(row.title).font(.system(size: WorkspaceStyle.PaneText.body))
                    Text(row.status)
                        .font(.system(size: WorkspaceStyle.PaneText.minimum, design: .monospaced))
                        .foregroundStyle(.secondary)
                }
                .padding(9)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 8).fill(WorkspaceStyle.paneChrome))
            }
            Spacer()
        }
        .frame(width: 260)
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 10).fill(WorkspaceStyle.document))
    }
}

/// One task, opened: what is understood now, and how it came to be understood.
///
/// The two halves are drawn as two halves on purpose. Above the divider is the
/// mutable present; below it is the record, which is append-only and which
/// this card offers nothing at all to change. There is no menu on a note here,
/// and there must never be one — correcting the record is a NEW note carrying
/// `supersedes`, which is `farcooler task note --supersedes`.
private struct TaskCard: View {
    let row: TaskRow
    let detail: TaskDetailModel
    let onClose: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(row.key)
                    .font(.system(size: WorkspaceStyle.PaneText.title, design: .monospaced))
                    .foregroundStyle(.secondary)
                Text(row.title).font(.headline)
                Spacer()
                Text(row.status.title)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                    .foregroundStyle(.secondary)
                Button("Done", action: onClose).keyboardShortcut(.defaultAction)
            }
            .padding(14)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    understanding
                    Divider()
                    record
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(minWidth: 560, minHeight: 480)
        .background(WorkspaceStyle.document)
    }

    @ViewBuilder private var understanding: some View {
        if let waiting = row.blockedSummary {
            Text(waiting)
                .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                .foregroundStyle(.orange)
            ForEach(row.blockedBy, id: \.key) { block in
                if !block.reason.isEmpty {
                    Text("\(block.key) — \(block.reason)")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                }
            }
        }
        if let note = row.stalenessNote(at: Date()) {
            Text(note)
                .font(.system(size: WorkspaceStyle.PaneText.body))
                .foregroundStyle(.orange)
        }
        if !row.intent.isEmpty {
            section("Intent") {
                Text(row.intent).font(.system(size: WorkspaceStyle.PaneText.body))
            }
        }
        if !row.acceptance.isEmpty {
            section("Acceptance") {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(row.acceptance) { line in
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Image(systemName: line.met ? "checkmark.circle.fill" : "circle")
                                .foregroundStyle(line.met ? Color.accentColor : .secondary)
                            Text(line.text).font(.system(size: WorkspaceStyle.PaneText.body))
                        }
                    }
                }
            }
        }
        if !row.constraints.isEmpty {
            section("Constraints") {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(row.constraints, id: \.self) { text in
                        Text("• \(text)").font(.system(size: WorkspaceStyle.PaneText.body))
                    }
                }
            }
        }
    }

    @ViewBuilder private var record: some View {
        section("Record") {
            VStack(alignment: .leading, spacing: 10) {
                if detail.notes.isEmpty {
                    Text("Nothing written down yet.")
                        .font(.system(size: WorkspaceStyle.PaneText.body))
                        .foregroundStyle(.secondary)
                }
                ForEach(detail.notes) { note in
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(spacing: 6) {
                            Text(note.kind.title)
                                .font(
                                    .system(
                                        size: WorkspaceStyle.PaneText.secondary,
                                        weight: .semibold)
                                )
                                .foregroundStyle(
                                    note.kind.isMachineWritten ? Color.secondary : Color.primary)
                            Text(note.byline)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(.secondary)
                            Text(note.at, style: .relative)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(.secondary)
                            if note.supersedes != nil {
                                Text("Replaces an earlier entry")
                                    .font(.system(size: WorkspaceStyle.PaneText.minimum))
                                    .foregroundStyle(.secondary)
                            }
                        }
                        Text(note.body)
                            .font(.system(size: WorkspaceStyle.PaneText.body))
                            .textSelection(.enabled)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                // Never silently: an entry this build cannot name is still an
                // entry, and the record's claim is that nothing in it is lost.
                if detail.unreadableNotes > 0 {
                    Text(unreadableSentence)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private var unreadableSentence: String {
        let n = detail.unreadableNotes
        return n == 1
            ? "1 more entry was written in a form this version can’t show."
            : "\(n) more entries were written in a form this version can’t show."
    }

    @ViewBuilder private func section<Content: View>(
        _ title: String, @ViewBuilder _ content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(title)
                .font(WorkspaceStyle.sectionTitle)
                .foregroundStyle(.secondary)
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
