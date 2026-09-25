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
    /// The generation of THIS repository's board this store has already acted
    /// on, so an event that arrives while a read is in flight is not read
    /// twice — and an event about another repository is not read at all.
    private var seenGeneration = 0

    init(client: DaemonClient, repository: Repository) {
        self.client = client
        self.repository = repository
        self.seenGeneration = client.boardGeneration(for: repository.id)
    }

    /// Whether the window should go on holding this store, given the runners
    /// it has now.
    ///
    /// No, once its client is not one of them — the runner was removed, or
    /// left and came back as a new client — because a store held over talks
    /// to a connection nobody is answering, and holds that client alive.
    /// No, once its runner is connected, has listed its projects, and this
    /// one isn't among them. Yes otherwise, and in particular while the
    /// runner is reconnecting: "the project is gone" can't be told from "the
    /// runner isn't answering" then, which is `missingBoardSentence`'s rule.
    func isHeld(by clients: [String: DaemonClient]) -> Bool {
        guard clients.values.contains(where: { $0 === client }) else { return false }
        guard client.state == .connected, client.repositoriesListed else { return true }
        return client.repositories.contains { $0.id == repository.id }
    }

    /// A number that moves whenever this repository's board may have moved:
    /// its own `task` events, and every reconnection. See
    /// `DaemonClient.boardGeneration(for:)`.
    var generation: Int { client.boardGeneration(for: repository.id) }

    /// Set when a read was asked for while one was already in flight.
    ///
    /// The in-flight read looks at this when it lands and reads once more,
    /// so a burst of events costs one `task list` running plus one after it,
    /// never one per event. SwiftUI cancelling a `.task` does not stop a
    /// `task list` already launched — `runRaw` waits on a `Process` — so
    /// before this, five writes in a second were five processes racing, and
    /// whichever finished last drew the board, older or not.
    private var readAgain = false

    /// The first read, once, however many views ask for it.
    ///
    /// The sidebar row and the board in the main area hold the same store and
    /// both appear at once when the row is selected; without the `reading`
    /// check each would launch its own `task list` for the same board.
    func readIfNeverRead() async {
        guard !hasRead, !reading else { return }
        await reload()
    }

    /// Re-read the whole board.
    ///
    /// Whole, and not a delta, because the daemon's event carries none — three
    /// editors move this state at once, and a client applying deltas would
    /// need to be right about all three. `task list` answers a board in one
    /// call, which is what makes re-reading the cheap answer.
    ///
    /// One at a time per store. A call that arrives while a read is running
    /// marks the board to be read again and returns; the running read then
    /// reads once more when it lands. So results land in the order they were
    /// asked for, and an older board can never be drawn over a newer one.
    ///
    /// Returns whether THIS call did the reading, which is what tells
    /// `reloadIfMoved` whether it is the one that should refresh an open card.
    @discardableResult
    func reload() async -> Bool {
        if reading {
            readAgain = true
            return false
        }
        reading = true
        defer { reading = false }
        repeat {
            readAgain = false
            await readOnce()
        } while readAgain
        return true
    }

    private func readOnce() async {
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
        guard generation != seenGeneration else { return }
        seenGeneration = generation
        // Only the call that read refreshes an open card. The ones folded into
        // a read already running would each launch a `task show` of their own
        // for the same card.
        guard await reload(), let opened else { return }
        await open(opened)
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

/// One pane that is working a task, with the workspace it is in.
///
/// Carried together because going to it needs both: `Selection.terminal`
/// names the workspace and the host as well as the terminal, and a pane found
/// on its own would have to be looked up again to learn where it lives.
struct BoardPane: Identifiable, Equatable {
    let terminal: Terminal
    let workspace: Workspace

    var id: String { terminal.id }

    /// What a menu item offering this pane says: the pane, then where it is.
    /// "claude in fix-reconnect", because two agents on one task are usually
    /// the same program, and the workspace is what tells them apart.
    ///
    /// Numbered the way the sidebar numbers it — "claude 2 in fix-reconnect"
    /// — when the workspace holds two alike, because `dispatch --again` can
    /// put the second agent in the same lane and two identical menu items
    /// would be a coin toss. See `Workspace.ordinals()`.
    var title: String {
        "\(terminal.displayName(ordinal: workspace.ordinals()[terminal.id])) in \(workspace.task)"
    }

    /// Menu titles for several panes, told apart even where their names are
    /// not: two panes the agent titled identically get their short ids.
    static func titles(_ panes: [BoardPane]) -> [String] {
        let plain = panes.map(\.title)
        let counts = Dictionary(plain.map { ($0, 1) }, uniquingKeysWith: +)
        return zip(panes, plain).map { pane, title in
            counts[title, default: 0] > 1 ? "\(title) (\(pane.terminal.short))" : title
        }
    }

    /// Where going to `pane` should land, looked up again in the fleet as it
    /// is NOW rather than as it was when the card drew.
    ///
    /// A menu is open while the fleet moves under it, so the pane can have
    /// exited and been reaped by the time it is chosen. Then: its workspace,
    /// if that is still there, and nil — stay on the board and say so — if
    /// neither is.
    static func landing(for pane: BoardPane, in fleet: [Workspace]) -> ContentView.Selection? {
        let host = pane.workspace.host ?? ""
        guard
            let workspace = fleet.first(where: {
                ($0.host ?? "") == host && $0.id == pane.workspace.id
            })
        else { return nil }
        if workspace.terminals.contains(where: { $0.id == pane.terminal.id }) {
            return .terminal(host: host, workspace: workspace.id, terminal: pane.terminal.id)
        }
        return .workspace(host: host, id: workspace.id)
    }
}

/// Every pane on a board's runner, and whether that runner says which pane
/// works which task.
///
/// A value handed to the board by the window, which is what holds the fleet.
/// The rule for which of these is working a card is AgentKit's
/// (`TaskAgentLink.isWorking`); this only pairs each pane with its workspace
/// so that the answer is somewhere you can go.
struct BoardAgents {
    /// The runner's workspaces, as the sidebar has them.
    var workspaces: [Workspace]
    /// Whether the runner advertises `terminal_task`. Without it no pane
    /// carries a task, and the board makes no claim either way.
    var runnerRecordsTasks: Bool

    static let none = BoardAgents(workspaces: [], runnerRecordsTasks: false)

    /// The panes a board may speak of on one runner — or none, which is
    /// "can't say": no pills, no "No Agent", and no count in the sidebar.
    ///
    /// Two gates. The runner has to record which pane works which task
    /// (`terminal_task`), and it has to be connected right now. Anything
    /// else — connecting, reconnecting, unreachable, not installed — means
    /// the workspaces are the last ones read before the link went, kept so
    /// the sidebar stays put, and the agents in them may have exited since.
    ///
    /// `.connected` and not `state.refusal == nil`: a dead runner spends most
    /// of an outage in `.reconnecting` between attempts, and that gate let the
    /// frozen pills blink back on for every one of them. `FleetStore.reading`
    /// counts the status bar's live panes by the same rule.
    static func on(
        _ workspaces: [Workspace], state: HostState, build: DaemonBuild?
    ) -> BoardAgents {
        guard state == .connected, build?.can("terminal_task") == true else { return .none }
        return BoardAgents(workspaces: workspaces, runnerRecordsTasks: true)
    }

    private var panes: [BoardPane] {
        workspaces.flatMap { ws in ws.terminals.map { BoardPane(terminal: $0, workspace: ws) } }
    }

    /// The panes working `row`, in sidebar order. Empty on a runner that
    /// doesn't record tasks, whatever its panes say.
    func live(for row: TaskRow) -> [BoardPane] {
        guard runnerRecordsTasks else { return [] }
        let working = Set(row.livePanes(in: workspaces.flatMap(\.terminals)).map(\.id))
        return panes.filter { working.contains($0.id) }
    }

    func presence(for row: TaskRow) -> TaskAgentPresence {
        row.agentPresence(livePanes: live(for: row).count, runnerRecordsTasks: runnerRecordsTasks)
    }

    /// How many of `board`'s tasks have an agent on them — the sidebar row's
    /// quiet count.
    func tasksWithAgents(on board: TaskBoardModel) -> Int {
        guard runnerRecordsTasks else { return 0 }
        return board.tasksWithLiveAgents(in: workspaces.flatMap(\.terminals))
    }
}

/// The board itself, in the main area of the window.
///
/// Not a sheet any more. A sheet sat over the agents it was orchestrating, so
/// going to one closed the board and coming back meant opening it again; here
/// it is a place in the sidebar like any workspace, and ⇧⌘B or a click on its
/// row brings it back.
struct TaskBoardView: View {
    @ObservedObject var store: TaskBoardStore
    @ObservedObject var client: DaemonClient
    let agents: BoardAgents
    /// Go to a pane working a task. The window's, because only the window can
    /// change what is selected.
    let onGoTo: (BoardPane) -> Void

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
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(WorkspaceStyle.canvas)
        // The board only moves when a runner says THIS board did. Polling a
        // board that nothing is touching would be the shape this app spent a
        // release removing.
        .task(id: store.generation) { await store.reloadIfMoved() }
        // Keyed on the store, not run once per view: a runner removed and
        // added back gets a new client, so the window hands this view a new
        // store in the same place, and a `.task` with no id would never read
        // it — the board would sit on empty columns until the next event.
        .task(id: ObjectIdentifier(store)) { await store.readIfNeverRead() }
        // A card is open only while its board is on screen. A board that goes
        // away under an open card — its project removed, its runner gone, or
        // a command that selected something else — would otherwise leave the
        // card set on a store the sidebar row still holds: re-read with a
        // `task show` on every event nobody sees, and back on screen the next
        // time the board is.
        .onDisappear { store.opened = nil }
        .sheet(item: $store.opened) { row in
            TaskCard(
                row: row, detail: store.detail, agents: agents.live(for: row),
                onGoTo: { pane in
                    store.opened = nil
                    onGoTo(pane)
                },
                onClose: { store.opened = nil })
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
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(WorkspaceStyle.paneChrome)
    }

    private var columns: some View {
        ScrollView(.horizontal) {
            HStack(alignment: .top, spacing: 12) {
                ForEach(store.board.columns) { column in
                    TaskColumnView(column: column, store: store, agents: agents, onGoTo: onGoTo)
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
    let agents: BoardAgents
    let onGoTo: (BoardPane) -> Void

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
                        TaskCardRow(
                            row: row, prominent: leads, store: store,
                            live: agents.live(for: row), presence: agents.presence(for: row),
                            onGoTo: onGoTo)
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
    /// The panes working this task, and what the card says about them. Both
    /// decided by AgentKit's rule; see `BoardAgents`.
    let live: [BoardPane]
    let presence: TaskAgentPresence
    let onGoTo: (BoardPane) -> Void

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
            // How far along it is, and who is on it: the two things a card
            // says about the work rather than about the task.
            if row.acceptanceProgress != nil || presence.title != nil {
                HStack(alignment: .center, spacing: 6) {
                    if let progress = row.acceptanceProgress {
                        AcceptanceProgressLabel(progress: progress)
                    }
                    Spacer(minLength: 0)
                    AgentPill(live: live, presence: presence, onGoTo: onGoTo)
                }
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
            // Going somewhere writes nothing, so it is not a `BoardAction`:
            // that list is for writes, and `rewritesTheRecord` walks it.
            GoToAgentItems(live: live, onGoTo: onGoTo)
            if !live.isEmpty { Divider() }
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

/// "2 of 5", or "All 5 met" in the accent color once every line holds.
private struct AcceptanceProgressLabel: View {
    let progress: TaskAcceptanceProgress

    var body: some View {
        HStack(spacing: 3) {
            Image(systemName: progress.isComplete ? "checkmark.circle.fill" : "checkmark.circle")
            Text(progress.sentence).monospacedDigit()
        }
        .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: progress.isComplete ? .medium : .regular))
        .foregroundStyle(progress.isComplete ? Color.accentColor : Color.secondary)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Acceptance: \(progress.sentence)")
    }
}

/// The way from a card to the agent working it.
///
/// A pill for one pane, the same pill as a menu for several, and a quiet
/// "No Agent" for a task in progress with nobody on it. The status mark is the
/// pane's own, drawn by `StatusGlyph` like the sidebar row it leads to — so an
/// agent waiting on a question is amber here too, and the card says which
/// agent needs you before you open anything.
private struct AgentPill: View {
    let live: [BoardPane]
    let presence: TaskAgentPresence
    let onGoTo: (BoardPane) -> Void

    @State private var hovering = false

    var body: some View {
        switch presence {
        case .unsaid:
            EmptyView()
        case .noAgent:
            Text(presence.title ?? "")
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.tertiary)
        case .agents:
            if live.count == 1, let pane = live.first {
                Button { onGoTo(pane) } label: { label(for: [pane]) }
                    .buttonStyle(.plain)
                    .help("Go to \(pane.title)")
            } else {
                Menu {
                    GoToAgentItems(live: live, onGoTo: onGoTo)
                } label: {
                    label(for: live)
                }
                // `.button` with a plain button style, so the menu draws the
                // same capsule as the single pill instead of AppKit's own
                // borderless chrome.
                .menuStyle(.button)
                .buttonStyle(.plain)
                .menuIndicator(.hidden)
                .fixedSize()
                .help("Go to one of the agents on this task")
            }
        }
    }

    private func label(for panes: [BoardPane]) -> some View {
        HStack(spacing: 4) {
            if let status = Status.mostUrgent(in: panes.map(\.terminal.status)) ?? panes.first?.terminal.status {
                StatusGlyph(status: status, inAppDiameter: 6)
            }
            Text(presence.title ?? "")
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
            Image(systemName: panes.count == 1 ? "arrow.right" : "chevron.down")
                .font(.system(size: 7.5, weight: .bold))
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 7)
        .padding(.vertical, 2.5)
        .background(
            Capsule().fill(Color.primary.opacity(hovering ? 0.12 : 0.07)))
        .contentShape(Capsule())
        .onHover { hovering = $0 }
        .animation(Motion.snap, value: hovering)
    }
}

/// "Go to Agent", or one "Go to …" per pane when several are on the task.
///
/// Shared by the card's context menu and the pill's menu so the two can't
/// come to list different panes.
private struct GoToAgentItems: View {
    let live: [BoardPane]
    let onGoTo: (BoardPane) -> Void

    var body: some View {
        if live.count == 1, let pane = live.first {
            Button("Go to Agent") { onGoTo(pane) }
        } else if !live.isEmpty {
            Section("Go to Agent") {
                let titles = BoardPane.titles(live)
                ForEach(Array(live.enumerated()), id: \.element.id) { index, pane in
                    Button(titles[index]) { onGoTo(pane) }
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
    /// The panes working this task. Going to one closes the card first — the
    /// card is a sheet, and a sheet left up would sit over the pane you went
    /// to.
    let agents: [BoardPane]
    let onGoTo: (BoardPane) -> Void
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
                if agents.count == 1, let pane = agents.first {
                    Button("Go to Agent") { onGoTo(pane) }
                        .help("Go to \(pane.title)")
                } else if !agents.isEmpty {
                    Menu("Go to Agent") {
                        GoToAgentItems(live: agents, onGoTo: onGoTo)
                    }
                    .fixedSize()
                }
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
