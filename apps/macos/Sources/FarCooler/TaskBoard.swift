import AgentKit
import SwiftUI

// A workspace's board.
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

/// One workspace's board, as this window last read it.
///
/// One store per workspace per client, held by `ContentView` — see
/// `boardStore(for:client:host:)` there, which follows `changesStore(for:client:)`
/// exactly: a `DaemonClient` is dropped when its runner leaves, and a store
/// held over from the old one would go on talking to a connection nobody is
/// answering.
@MainActor
final class TaskBoardStore: ObservableObject {
    @Published private(set) var board: TaskBoardModel = .empty
    @Published private(set) var detail: TaskDetailModel = .empty
    /// The card `detail` and `question` were read for. They are single slots,
    /// and a column that has just switched to another task renders once
    /// before `open` blanks them: drawn through `detail(for:)` and
    /// `question(for:)`, that frame is empty rather than the last task's.
    private var readID: String?
    /// `detail` if it is `id`'s record, else empty.
    func detail(for id: String) -> TaskDetailModel { readID == id ? detail : .empty }

    /// `question` if it is `id`'s, else nil.
    func question(for id: String) -> TaskQuestion? { readID == id ? question : nil }

    /// The card that is open, or nil for the board alone.
    @Published var opened: TaskRow?
    /// What choosing a card does: the window opens it in the workspace's
    /// task column. Nil opens it here, which is all a test needs.
    var onChoose: ((TaskRow) -> Void)?

    /// A card was clicked: open it where the window puts tasks.
    func choose(_ row: TaskRow) {
        if let onChoose { onChoose(row) } else { Task { await open(row) } }
    }
    // MARK: - Since you were last here

    /// When this person last left this workspace's board, as it stood when
    /// they came back to it. Held still while they read: `markVisited` writes
    /// the new moment to disk, and this changes only at `beginVisit`, so the
    /// summary doesn't empty itself under their eyes.
    @Published private(set) var visitBaseline: Date?
    /// Decisions and findings read for the summary, by task id.
    @Published private(set) var summaryNotes: [String: [TaskNoteRow]] = [:]
    private var noteCache: [String: (updatedAt: Date?, notes: [TaskNoteRow])] = [:]

    /// A visit begins: the last one's end is the summary's "last visit".
    func beginVisit(in defaults: UserDefaults = .standard) {
        visitBaseline = BoardVisit.read(host: hostKey, workspace: workspace.id, from: defaults)
    }

    /// A visit ends — the person left the workspace or the app. Called then,
    /// never continuously. Anything that counts as using the orchestrator or
    /// board (typing into its pane, acting on a card) can call it too.
    func markVisited(in defaults: UserDefaults = .standard, now: Date = Date()) {
        BoardVisit.write(now, host: hostKey, workspace: workspace.id, in: defaults)
    }

    /// Read the records of the few tasks that moved since `since`, so the
    /// summary can list their decisions and findings. One `task show` each,
    /// remembered until the task's `updatedAt` moves.
    func readSummaryNotes(since: Date) async {
        let picked = BoardSummary.noteCandidates(rows: board.rows, since: since)
        for row in picked where noteCache[row.id]?.updatedAt != row.updatedAt {
            let (data, _) = await client.taskDetail(key: row.key, repository: repositoryID)
            guard let data, let read = try? TaskDetailModel.decode(data) else { continue }
            noteCache[row.id] = (row.updatedAt, read.notes)
        }
        summaryNotes = Dictionary(
            uniqueKeysWithValues: picked.compactMap { row in
                noteCache[row.id].map { (row.id, $0.notes) }
            })
    }

    /// Whether a read has ever come back.
    ///
    /// Separate from "the board is empty", which is a real and different
    /// answer: a repository with no tasks on it should say so, and one that
    /// has not been read yet must not.
    @Published private(set) var hasRead = false
    /// Tasks an agent is being started for, by id. A start makes a worktree and
    /// an agent over several runner calls, and a second one made meanwhile
    /// would make `name-2` beside it for the same task.
    @Published private(set) var starting: Set<String> = []
    @Published private(set) var reading = false
    /// What to say when a read or a move did not work.
    ///
    /// This app's sentence, never the runner's. The CLI's own stderr is
    /// written for whoever is reading a terminal — it names methods and Rust
    /// types — and putting it on a board would be the one thing the refusal
    /// vocabulary exists to prevent. It is dropped rather than tucked into a
    /// tooltip, because a tooltip is still a screen.
    @Published private(set) var trouble: String?

    /// The workspace whose board this is. An implicit one — a runner without
    /// workspaces — is its repository's whole board.
    let workspace: WorkspaceSummary
    /// Held, and readable, so the window can tell a store built against a
    /// dropped connection from one built against the live link — see
    /// `boardStore(for:client:)` in `ContentView`, which is the same identity
    /// check `changesStore(for:client:)` makes for the same reason.
    let client: DaemonClient
    /// The generation of THIS board this store has already acted on, so an
    /// event that arrives while a read is in flight is not read twice — and
    /// an event about another board is not read at all.
    private var seenGeneration = 0

    init(client: DaemonClient, workspace: WorkspaceSummary) {
        self.client = client
        self.workspace = workspace
        self.seenGeneration = client.boardGeneration(for: workspace)
    }

    /// The repository this board is in, by its uuid: what `task show` and
    /// `task set` are scoped by. An implicit workspace's id is its
    /// repository's.
    var repositoryID: String { workspace.repository ?? workspace.id }

    /// What the board's title says: the workspace, or for a runner without
    /// workspaces the repository, as it was before there were several.
    var title: String {
        guard workspace.isImplicit else { return workspace.name }
        return client.repositories.first { $0.id == repositoryID }?.displayName ?? "Board"
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
    /// And no, once that runner lists workspaces and this one isn't among
    /// them: it was deleted. A runner that lists none can't say.
    func isHeld(by clients: [String: DaemonClient]) -> Bool {
        guard clients.values.contains(where: { $0 === client }) else { return false }
        guard client.state == .connected, client.repositoriesListed else { return true }
        guard client.repositories.contains(where: { $0.id == repositoryID }) else { return false }
        guard !workspace.isImplicit, let listed = client.fleet.workspaces else { return true }
        return listed.contains { $0.id == workspace.id }
    }

    /// A number that moves whenever this board may have moved: its own
    /// `task` events, and every reconnection. See
    /// `DaemonClient.boardGeneration(for:)`.
    var generation: Int { client.boardGeneration(for: workspace) }

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
        let (data, _) = await client.taskBoard(
            repository: repositoryID, workspace: workspace.boardWorkspace)
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
        guard await reload(), opened != nil else { return }
        await refreshOpened()
    }

    /// Open one card: its record, and what it is waiting on.
    ///
    /// A second call, because `task list` carries neither — one call for a
    /// whole board is what makes surveying it cheap, and the record is the
    /// expensive half.
    ///
    /// A different card starts empty. The card already open is read again in
    /// place instead (`refreshOpened`): the board re-reads it on every task
    /// write it hears of, most of them to other cards, and emptying it for
    /// each one tore down the answer being typed into it.
    func open(_ row: TaskRow) async {
        if opened?.id != row.id {
            opened = row
            readID = row.id
            detail = .empty
            question = nil
        }
        await read(row)
    }

    /// Read the open card again, keeping what it shows until the new read
    /// lands.
    func refreshOpened() async {
        guard let opened else { return }
        await read(opened)
    }

    private func read(_ row: TaskRow) async {
        let (data, _) = await client.taskDetail(key: row.key, repository: repositoryID)
        // A card closed, or another opened, while this was in flight.
        guard opened?.id == row.id else { return }
        guard let data, let read = try? TaskDetailModel.decode(data) else {
            trouble = "Far Cooler couldn’t read this task."
            return
        }
        trouble = nil
        readID = row.id
        detail = read
        let asked = TaskQuestion.open(in: data)
        // Assigned only when it changed, so an unchanged question keeps its
        // view, and the field in it.
        if asked != question { question = asked }
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
            key: row.key, to: status.rawValue, repository: repositoryID)
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

    /// The open card's question, when its record has one still waiting:
    /// what the card draws as Answer buttons. Read from the same `task show`
    /// as `detail`.
    @Published private(set) var question: TaskQuestion?

    /// What has been typed into Answer… and not sent, by question. The
    /// store's rather than the field's: the card is redrawn whenever its
    /// record is read again, and a draft held by the view went with it.
    /// Not published, so typing redraws the field and not the board.
    private var drafts: [String: String] = [:]

    func draft(for question: TaskQuestion) -> String { drafts[question.id] ?? "" }

    func setDraft(_ text: String, for question: TaskQuestion) {
        drafts[question.id] = text.isEmpty ? nil : text
    }

    /// This runner, as the board's per-device choices are keyed: its target,
    /// or `local` for this Mac.
    var hostKey: String { client.target.isEmpty ? "local" : client.target }

    /// Whether this board offers its writes: New Task…, and a question's
    /// Answer buttons. `TaskBoardWrites.offered`'s rule over this runner's
    /// build.
    var offersWrites: Bool { TaskBoardWrites.offered(by: client.daemonBuild) }

    /// File a task on this board: New Task…. True when it went on.
    ///
    /// Re-reads on the way back, as a move does, rather than drawing a card
    /// this window made up: the runner gives it its key.
    func createTask(title: String) async -> Bool {
        if let refused = await client.createTask(
            title: title, workspace: workspace.boardWorkspace, repository: repositoryID)
        {
            // The runner's words stay off the board; see `trouble`.
            _ = refused
            return false
        }
        await reload()
        return true
    }

    /// Start an agent for `row`, which has no worktree: New Task's own start
    /// path, then the task is put on the new lane and the board read again.
    /// Nil when it went; else a sentence for the window.
    func startAgent(for row: TaskRow, agent: String, undelivered: (@MainActor (String) -> Void)? = nil)
        async -> String?
    {
        guard !starting.contains(row.id) else { return nil }
        starting.insert(row.id)
        defer { starting.remove(row.id) }
        let outcome = await client.startAgent(
            for: row, repository: repositoryID, workspace: workspace.boardWorkspace, agent: agent,
            undelivered: undelivered)
        await reload()
        if opened?.id == row.id { await refreshOpened() }
        if case .failed(let sentence, _) = outcome { return sentence }
        return nil
    }

    /// Put `row` on an existing worktree. Nil when it went.
    func attach(_ row: TaskRow, toWorktree worktree: String) async -> String? {
        if await client.linkTask(key: row.key, worktree: worktree, repository: repositoryID) != nil {
            return "Far Cooler couldn’t attach that worktree to the task. Check that the runner is reachable, then try Open Worktree… again."
        }
        await reload()
        if opened?.id == row.id { await refreshOpened() }
        return nil
    }

    /// Answer a task's question with `body`: an option's text, or what was
    /// typed. True when it was written. The card is read again, so the
    /// answer shows in its record and the buttons go.
    func answer(_ row: TaskRow, with body: String) async -> Bool {
        if let refused = await client.answerDecision(
            key: row.key, body: body, repository: repositoryID)
        {
            _ = refused
            return false
        }
        if let question, opened?.id == row.id { setDraft("", for: question) }
        if opened?.id == row.id { await refreshOpened() }
        await reload()
        return true
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

/// One pane that is working a task, with the worktree it is in.
///
/// Carried together because going to it needs both: `Selection.terminal`
/// names the worktree and the host as well as the terminal, and a pane found
/// on its own would have to be looked up again to learn where it lives.
struct BoardPane: Identifiable, Equatable {
    let terminal: Terminal
    let worktree: Worktree

    var id: String { terminal.id }

    /// What a menu item offering this pane says: the pane, then where it is.
    /// "claude in fix-reconnect", because two agents on one task are usually
    /// the same program, and the worktree is what tells them apart.
    ///
    /// Numbered the way the sidebar numbers it — "claude 2 in fix-reconnect"
    /// — when the worktree holds two alike, because `dispatch --again` can
    /// put the second agent in the same lane and two identical menu items
    /// would be a coin toss. See `Worktree.ordinals()`.
    var title: String {
        "\(terminal.displayName(ordinal: worktree.ordinals()[terminal.id])) in \(worktree.task)"
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
    /// exited and been reaped by the time it is chosen. Then: its worktree,
    /// if that is still there, and nil — stay on the board and say so — if
    /// neither is. Either lands as `WorkspaceSelection` says a pane or a
    /// worktree does.
    static func landing(for pane: BoardPane, in fleet: Fleet) -> ContentView.Selection? {
        let host = pane.worktree.host ?? ""
        guard let worktree = WorkspaceSelection.worktree(host: host, id: pane.worktree.id, in: fleet)
        else { return nil }
        let live = worktree.terminals.contains(where: { $0.id == pane.terminal.id })
        return WorkspaceSelection.landing(in: worktree, terminal: live ? pane.terminal.id : nil, fleet: fleet)
    }
}

/// Every pane on a board's runner, and whether that runner says which pane
/// works which task.
///
/// A value handed to the board by the window, which is what holds the fleet.
/// The rule for which of these is working a card is AgentKit's
/// (`TaskAgentLink.isWorking`); this only pairs each pane with its worktree
/// so that the answer is somewhere you can go.
struct BoardAgents {
    /// The runner's worktrees, as the sidebar has them.
    var worktrees: [Worktree]
    /// Whether the runner advertises `terminal_task`. Without it no pane
    /// carries a task, and the board makes no claim either way.
    var runnerRecordsTasks: Bool

    static let none = BoardAgents(worktrees: [], runnerRecordsTasks: false)

    /// The panes a board may speak of on one runner — or none, which is
    /// "can't say": no pills, no "No Agent", and no count in the sidebar.
    ///
    /// Two gates. The runner has to record which pane works which task
    /// (`terminal_task`), and it has to be connected right now. Anything
    /// else — connecting, reconnecting, unreachable, not installed — means
    /// the worktrees are the last ones read before the link went, kept so
    /// the sidebar stays put, and the agents in them may have exited since.
    ///
    /// `.connected` and not `state.refusal == nil`: a dead runner spends most
    /// of an outage in `.reconnecting` between attempts, and that gate let the
    /// frozen pills blink back on for every one of them. `FleetStore.reading`
    /// counts the status bar's live panes by the same rule.
    static func on(
        _ worktrees: [Worktree], state: HostState, build: DaemonBuild?
    ) -> BoardAgents {
        // The rule is AgentKit's, so this board and the phone's cannot drift.
        guard TaskAgentLink.speaksOfAgents(connected: state == .connected, build: build)
        else { return .none }
        return BoardAgents(worktrees: worktrees, runnerRecordsTasks: true)
    }

    private var panes: [BoardPane] {
        worktrees.flatMap { ws in ws.terminals.map { BoardPane(terminal: $0, worktree: ws) } }
    }

    /// The panes working `row`, in sidebar order. Empty on a runner that
    /// doesn't record tasks, whatever its panes say.
    func live(for row: TaskRow) -> [BoardPane] {
        guard runnerRecordsTasks else { return [] }
        let working = Set(row.livePanes(in: worktrees.flatMap(\.terminals)).map(\.id))
        return panes.filter { working.contains($0.id) }
    }

    func presence(for row: TaskRow) -> TaskAgentPresence {
        row.agentPresence(livePanes: live(for: row).count, runnerRecordsTasks: runnerRecordsTasks)
    }

    /// How many of `board`'s tasks have an agent on them — the sidebar row's
    /// quiet count.
    func tasksWithAgents(on board: TaskBoardModel) -> Int {
        guard runnerRecordsTasks else { return 0 }
        return board.tasksWithLiveAgents(in: worktrees.flatMap(\.terminals))
    }
}

/// Which of the board's writes this connection may make.
///
/// New Task… and a question's Answer buttons are Control-scope writes
/// (`task.create`, `task.note`). A connection granted only Read sees the
/// board without them, which is the rule Needs You follows too (spec §2.5).
/// Anything but `read` offers them, `unspecified` included: that is what a
/// runner newer than this build answers for a grant it has no word for, and
/// what the Mac's own shell key reads as (`DaemonBuild.mayAdministerRunner`).
///
/// **This gate can't close on a Mac today.** The Mac reaches a runner over
/// its own shell key, which the daemon reads as host_admin, and `status
/// --json` carries no scope, so `grantedScope` is `unspecified` here and the
/// writes are always offered. `NewTaskTests` pins the rule, not the view; it
/// becomes reachable when the Mac learns its grant.
enum TaskBoardWrites {
    static func offered(by build: DaemonBuild?) -> Bool {
        build?.grantedScope != "read"
    }

    /// The most a task's title may hold, in Unicode scalars: the daemon's
    /// limit (`checked_title`, `crates/daemon/src/task_ops.rs:163`).
    static let titleLimit = 200

    /// Whether the daemon will take `title`, measured as it measures it:
    /// trimmed, not empty, and at most `titleLimit` scalars. Scalars rather
    /// than characters, because a flag or a family emoji is one character and
    /// several scalars, and counting characters let through titles the runner
    /// then refused.
    static func titleFits(_ title: String) -> Bool {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return !trimmed.isEmpty && trimmed.unicodeScalars.count <= titleLimit
    }
}

/// The board itself, in the main area of the window.
///
/// Not a sheet any more. A sheet sat over the agents it was orchestrating, so
/// going to one closed the board and coming back meant opening it again; here
/// it is a place in the sidebar like any worktree, and ⇧⌘B or a click on its
/// row brings it back.
///
/// **One form, the status-sectioned list**, on the board column's grid
/// (`ColumnGrid`): each section's chevron at column A, its title at B, its
/// count trailing, and its cards' text at B. Every status is drawn, an empty
/// one as a collapsed "Backlog 0". There was a kanban too, for a wide board,
/// with a toggle in the header; the owner removed both (ov-83), since the
/// board almost always sits in a narrow column.
struct TaskBoardView: View {
    @ObservedObject var store: TaskBoardStore
    @ObservedObject var client: DaemonClient
    let agents: BoardAgents
    /// How many decisions are waiting on the person, from the needs-you
    /// store (`WorkspaceCounts.decisions`), not from the Needs Decision
    /// column: answering a decision leaves its task in that column (spec
    /// §2.2), so the column outlives the question.
    let waiting: Int
    /// Go to a pane working a task. The window's, because only the window can
    /// change what is selected.
    let onGoTo: (BoardPane) -> Void
    /// Where the collapsed sections are kept. The app's own defaults,
    /// except in a test.
    let defaults: UserDefaults

    /// The list's collapsed sections: read from `defaults` in `init`, and
    /// again when the view is handed another board.
    @State private var collapsed: Set<TaskStatus>
    @State private var newTaskOpen = false

    init(
        store: TaskBoardStore, client: DaemonClient, agents: BoardAgents, waiting: Int = 0,
        onGoTo: @escaping (BoardPane) -> Void, defaults: UserDefaults = .standard
    ) {
        self.store = store
        self.client = client
        self.agents = agents
        self.waiting = waiting
        self.onGoTo = onGoTo
        self.defaults = defaults
        _collapsed = State(
            initialValue: BoardForm.collapsed(
                host: store.hostKey, workspace: store.workspace.id, from: defaults))
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            if store.hasRead {
                BoardSummaryStrip(store: store, defaults: defaults)
                    .id(ObjectIdentifier(store))
                Divider()
            }
            if !store.hasRead && store.reading {
                centered { ProgressView() }
            } else if let trouble = store.trouble, !store.hasRead {
                centered {
                    VStack(spacing: 10) {
                        Text(trouble)
                        Button("Try Again") { Task { await store.reload() } }
                    }
                }
            } else {
                list
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
        // This board's choices, read again whenever the view is handed
        // another board.
        // A visit begins when the board comes up and ends when the person
        // leaves it: the workspace, the window, or the app. The summary's
        // baseline is read at the start and written at the end, so it holds
        // still while they read.
        .onAppear { store.beginVisit(in: defaults) }
        .onDisappear { store.markVisited(in: defaults) }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.willResignActiveNotification)) { _ in
            store.markVisited(in: defaults)
        }
        // Deliberately no re-read when the app comes back: a quick ⌘-Tab out and
        // in would otherwise shrink "Since Last Visit" to seconds. The stamp
        // written above is read at the next appear, after the person has
        // left the workspace.
        .onChange(of: remembered) { old, key in
            BoardVisit.write(Date(), host: old.host, workspace: old.workspace, in: defaults)
            store.beginVisit(in: defaults)
            collapsed = BoardForm.collapsed(host: key.host, workspace: key.workspace, from: defaults)
        }
    }

    /// Which board's choices are on screen: the runner and the workspace.
    private struct Remembered: Equatable {
        var host: String
        var workspace: String
    }

    private var remembered: Remembered {
        Remembered(host: store.hostKey, workspace: store.workspace.id)
    }

    private func centered<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack { Spacer(); content(); Spacer() }.frame(maxWidth: .infinity)
    }

    // MARK: - Header

    private var header: some View {
        BoardHeader(
            title: store.title, waiting: waiting, reading: store.reading,
            trouble: store.hasRead ? store.trouble : nil, offersWrites: store.offersWrites,
            newTaskOpen: $newTaskOpen,
            onCreate: { title in await store.createTask(title: title) },
            onRefresh: { Task { await store.reload() } })
    }

    /// The waiting pill's short form, for a board too narrow for the
    /// sentence: "2 waiting".
    static func waitingShort(_ count: Int) -> String { BoardHeader.waitingShort(count) }

    // MARK: - The list

    private var list: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
                ForEach(store.board.sections) { section in
                    TaskListSection(
                        section: section,
                        expanded: BoardForm.isExpanded(section, collapsed: collapsed),
                        onToggle: { toggle(section.status) },
                        store: store, agents: agents, onGoTo: onGoTo)
                }
                if !store.board.unreadable.isEmpty {
                    UnreadableColumnView(rows: store.board.unreadable)
                }
            }
            // Measured from the board column's edge: the sections' chevrons
            // and the cards' edges at column A.
            .padding(.horizontal, ColumnGrid.a)
            .padding(.vertical, ColumnGrid.rhythm)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-list")
    }

    /// Open or close one section, and keep it that way on this device.
    private func toggle(_ status: TaskStatus) {
        if collapsed.contains(status) {
            collapsed.remove(status)
        } else {
            collapsed.insert(status)
        }
        BoardForm.setCollapsed(
            collapsed, host: store.hostKey, workspace: store.workspace.id, in: defaults)
    }
}

/// New Task…: a title, and the button that files it on this board.
///
/// Only the title, which is all `task create` needs and all a board card
/// shows; the intent and acceptance are the orchestrator's to write, or the
/// CLI's for anyone who wants them now.
struct NewTaskForm: View {
    let onCreate: (String) async -> Bool
    let onClose: () -> Void

    @State private var title = ""
    @State private var sending = false
    @State private var failed = false

    private var trimmed: String { title.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("New Task").font(.headline)
            TextField("Title", text: $title)
                .textFieldStyle(.roundedBorder)
                .frame(width: 320)
                .onSubmit(send)
            if !trimmed.isEmpty && !TaskBoardWrites.titleFits(trimmed) {
                Text("That title is too long. Shorten it to add the task.")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
            } else if failed {
                Text("Far Cooler couldn’t put that task on the board. Try again.")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
            }
            HStack {
                if sending { ProgressView().controlSize(.small) }
                Spacer()
                Button("Cancel", action: onClose).keyboardShortcut(.cancelAction)
                Button("Add Task", action: send)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSend)
            }
        }
        .padding(14)
    }

    private var canSend: Bool { !sending && TaskBoardWrites.titleFits(trimmed) }

    private func send() {
        guard canSend else { return }
        sending = true
        failed = false
        let text = trimmed
        Task {
            let filed = await onCreate(text)
            sending = false
            if filed { onClose() } else { failed = true }
        }
    }
}

/// One status in the list form: its header, and its cards when open.
///
/// An empty status is a header reading "Backlog 0" that can't open, so the
/// list says what isn't there as well as what is.
private struct TaskListSection: View {
    let section: TaskBoardColumn
    let expanded: Bool
    let onToggle: () -> Void
    @ObservedObject var store: TaskBoardStore
    let agents: BoardAgents
    let onGoTo: (BoardPane) -> Void
    @State private var showingAllDone = false

    /// Needs Decision is the one status waiting on the person reading, and
    /// the only one drawn in the accent color.
    private var leads: Bool { section.status == .needsDecision }

    var body: some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
            Button(action: onToggle) {
                // Chevron at column A, title at B, count trailing (ov-83).
                HStack(spacing: 0) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                        .foregroundStyle(.secondary)
                        .opacity(BoardForm.canExpand(section) ? 1 : 0.35)
                        .frame(width: ColumnGrid.step, alignment: .leading)
                        .gridMark("section", .chevron)
                    Text(section.title)
                        .font(WorkspaceStyle.sectionTitle)
                        .foregroundStyle(
                            section.count == 0
                                ? Color.secondary : leads ? Color.accentColor : Color.primary)
                        .gridMark("section", .text)
                    Spacer(minLength: SidebarGrid.gap)
                    Text("\(section.count)")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
                .frame(minHeight: ColumnGrid.rowHeight)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!BoardForm.canExpand(section))
            .animation(Motion.snap, value: expanded)
            .accessibilityLabel("\(section.title), \(section.count)")
            .accessibilityValue(BoardForm.canExpand(section) ? (expanded ? "Expanded" : "Collapsed") : "")
            .accessibilityIdentifier("board-section-\(section.id)")
            if expanded {
                VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
                    ForEach(section.visibleRows(showingAllDone: showingAllDone, now: Date())) { row in
                        TaskListRow(
                            row: row, prominent: leads, store: store,
                            live: agents.live(for: row), presence: agents.presence(for: row),
                            onGoTo: onGoTo)
                    }
                    ShowAllDoneButton(section: section, showingAll: $showingAllDone)
                }
            }
        }
    }
}

/// The quiet button under a Done column that is showing only the recent work.
/// Nothing for another status, or for a Done with nothing hidden.
private struct ShowAllDoneButton: View {
    let section: TaskBoardColumn
    @Binding var showingAll: Bool

    var body: some View {
        if section.status == .done, section.count > 0,
            showingAll || section.hidesDone(showingAllDone: false, now: Date())
        {
            Button(showingAll ? "Show Recent Done Only" : BoardDone.showAllTitle(total: section.count)) {
                showingAll.toggle()
            }
            .buttonStyle(.plain)
            .font(.system(size: WorkspaceStyle.PaneText.secondary))
            .foregroundStyle(.secondary)
            .gridMark("showAllDone", .text)
            // At column B, under the cards' text.
            .padding(.leading, ColumnGrid.step)
            .accessibilityIdentifier("board-show-all-done")
        }
    }
}

/// One card in the list, after the iPhone's row: its key, title, what it
/// asks of you, how long it has sat, how much of its acceptance holds, its
/// labels, and the way to its agent, which sits beside the words rather than
/// under them, since a list row has the width.
///
/// Its edge at column A and its text at B, under its section's title.
/// Internal rather than private for `BoardCardTickTests`, which draws it.
struct TaskListRow: View {
    let row: TaskRow
    let prominent: Bool
    @ObservedObject var store: TaskBoardStore
    let live: [BoardPane]
    let presence: TaskAgentPresence
    let onGoTo: (BoardPane) -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text(row.key)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .gridMark("card", .text)
                    // A stale row is visibly different, which is the
                    // board's whole job beyond showing state: a task sitting
                    // in `todo` that you assumed was in flight is the failure
                    // mode of the factory. On the board's tick, like the
                    // sentence and the border: a card crosses a day of
                    // silence on a quiet board with no data change to redraw
                    // it, and its three stale marks turn together.
                    BoardTick { now in
                        if row.staleness(at: now) == .stale {
                            Image(systemName: "clock.badge.exclamationmark")
                                .foregroundStyle(.orange)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        }
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
                if let call = row.callToAction {
                    Text(call)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                        .foregroundStyle(Color.accentColor)
                }
                HStack(spacing: 10) {
                    CardTimeLines(
                        row: row,
                        staleSize: WorkspaceStyle.PaneText.secondary,
                        timeSize: WorkspaceStyle.PaneText.minimum)
                    if let progress = row.acceptanceProgress {
                        AcceptanceProgressLabel(progress: progress)
                    }
                }
                if !row.labels.isEmpty {
                    Text(row.labels.joined(separator: " · "))
                        .font(.system(size: WorkspaceStyle.PaneText.minimum))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            AgentPill(live: live, presence: presence, onGoTo: onGoTo)
        }
        .padding(.horizontal, ColumnGrid.step)
        .padding(.vertical, ColumnGrid.rhythm)
        .background(RoundedRectangle(cornerRadius: 8).fill(WorkspaceStyle.paneChrome))
        .overlay(
            BoardTick { now in
                let stale = row.staleness(at: now) == .stale
                RoundedRectangle(cornerRadius: 8)
                    .strokeBorder(
                        stale ? Color.orange.opacity(0.55) : WorkspaceStyle.hairline,
                        lineWidth: stale ? 1 : 0.5)
            }
        )
        .contentShape(Rectangle())
        .onTapGesture { store.choose(row) }
        .contextMenu { TaskRowMenu(row: row, live: live, store: store, onGoTo: onGoTo) }
        .accessibilityIdentifier("board-row-\(row.key)")
    }
}

/// A card's context menu: the way to its agent, and the moves the model
/// offers.
private struct TaskRowMenu: View {
    let row: TaskRow
    let live: [BoardPane]
    @ObservedObject var store: TaskBoardStore
    let onGoTo: (BoardPane) -> Void

    var body: some View {
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

/// A card's two clock sentences, redrawn on the minute.
///
/// "Hasn’t moved in 3 days" in orange, or else the quiet "Updated 2h ago" /
/// "Added 3d ago" — both composed by AgentKit against the moment handed in.
/// The board itself redraws only when a task changes, so without a clock of
/// its own a card filed at six would still read "Added just now" at eleven
/// on a quiet board. `BoardTick` scoped to these two lines and nothing
/// else: a tick redraws the sentences, not the card or the board around it.
private struct CardTimeLines: View {
    let row: TaskRow
    let staleSize: CGFloat
    let timeSize: CGFloat

    var body: some View {
        BoardTick { now in
            if let note = row.stalenessNote(at: now) {
                Text(note)
                    .font(.system(size: staleSize))
                    .foregroundStyle(.orange)
            } else if let time = row.timeNote(at: now) {
                Text(time)
                    .font(.system(size: timeSize))
                    .foregroundStyle(.secondary)
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

    /// A section like the statuses above it: its heading at column B, and
    /// its rows as cards with their edges at A and their text at B.
    var body: some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Not On This Version").font(WorkspaceStyle.sectionTitle)
                    .gridMark("unreadable", .text)
                Text("This runner uses states this Far Cooler doesn’t have yet.")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.leading, ColumnGrid.step)
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
                .padding(.horizontal, ColumnGrid.step)
                .padding(.vertical, ColumnGrid.rhythm)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 8).fill(WorkspaceStyle.paneChrome))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// One task's card: what is understood now, the question it's waiting on,
/// and how it came to be understood.
///
/// The two halves are drawn as two halves on purpose. Above the divider is the
/// mutable present; below it is the record, which is append-only and which
/// this card offers nothing at all to change. There is no menu on a note here,
/// and there must never be one — correcting the record is a NEW note carrying
/// `supersedes`, which is `farcooler task note --supersedes`. Answering a
/// question is a new note too (`task note --kind answer`), never an edit.
///
/// Internal for `TaskCardTests`, which draws it. The task column hosts it.
struct TaskCard: View {
    let row: TaskRow
    let detail: TaskDetailModel
    let question: TaskQuestion?
    /// Whether this connection may answer: not on a read-scoped one. See
    /// `TaskBoardWrites`.
    let canAnswer: Bool
    /// Send an answer; true when it was written.
    let onAnswer: (String) async -> Bool
    /// Where Answer…'s unsent text is kept: the store, so it outlives this
    /// view, which is redrawn whenever the card's record is read again.
    var draft: Draft = .none

    /// Reads and writes an unsent answer, by question.
    struct Draft {
        var read: (TaskQuestion) -> String
        var write: (TaskQuestion, String) -> Void

        /// Nowhere: a card nobody can type into, or a test's.
        static var none: Draft { Draft(read: { _ in "" }, write: { _, _ in }) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            understanding
            if let offer = Self.offer(row: row, question: question, canAnswer: canAnswer) {
                QuestionAnswers(offer: offer, onAnswer: onAnswer, draft: draft)
                    .id(offer.question.id)
            }
            Divider()
            record
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// What a card offers for its question, which is all `QuestionAnswers`
    /// draws from.
    struct Offer: Equatable {
        var question: TaskQuestion
        /// Options drawn as buttons, in the order they were offered.
        var buttons: [String]
        /// Options past the buttons, in a More menu.
        var more: [String]
        /// Answer…, for a question that offered no options.
        var typed: Bool
    }

    /// The question a card shows and the answers it offers, or nil for none.
    ///
    /// - In Needs Decision only: a question still in the record of a task
    ///   someone has since moved on is history, not a request.
    /// - Three options at most as buttons, the rest in a menu; Answer… when
    ///   there are none (spec §2.5).
    /// - On a read-scoped connection, the question with nothing to press.
    static func offer(row: TaskRow, question: TaskQuestion?, canAnswer: Bool) -> Offer? {
        guard row.status == .needsDecision, let question else { return nil }
        guard canAnswer else { return Offer(question: question, buttons: [], more: [], typed: false) }
        return Offer(
            question: question, buttons: question.buttons, more: question.overflow,
            typed: question.options.isEmpty)
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
        CardTimeLines(
            row: row,
            staleSize: WorkspaceStyle.PaneText.body,
            timeSize: WorkspaceStyle.PaneText.secondary)
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
            VStack(alignment: .leading, spacing: 12) {
                if detail.notes.isEmpty {
                    Text("Nothing written down yet.")
                        .font(.system(size: WorkspaceStyle.PaneText.body))
                        .foregroundStyle(.secondary)
                }
                let paired = TaskNoteStyle.answersPaired(detail.notes)
                ForEach(detail.notes) { note in
                    TaskNoteView(note: note, pairedWithQuestion: paired.contains(note.id))
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

/// A question waiting on the person reading, and the ways to answer it:
/// `TaskCard.offer`'s, drawn and nothing decided here.
private struct QuestionAnswers: View {
    let offer: TaskCard.Offer
    let onAnswer: (String) async -> Bool
    let draft: TaskCard.Draft

    /// The answer on its way, which shows a spinner in place of the buttons.
    @State private var sending: String?
    @State private var failed = false
    @State private var writing = false
    @State private var typed = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Question")
                .font(WorkspaceStyle.sectionTitle)
                .foregroundStyle(Color.accentColor)
            Text(offer.question.body)
                .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                .textSelection(.enabled)
            if sending != nil {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Answering…")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                }
            } else if offer.typed {
                freeAnswer
            } else if !offer.buttons.isEmpty {
                HStack(spacing: 8) {
                    ForEach(offer.buttons, id: \.self) { option in
                        Button(option) { send(option) }
                    }
                    if !offer.more.isEmpty {
                        Menu("More") {
                            ForEach(offer.more, id: \.self) { option in
                                Button(option) { send(option) }
                            }
                        }
                        .fixedSize()
                    }
                }
            }
            if failed {
                Text("Your answer wasn’t sent. Try again.")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color.accentColor.opacity(0.07)))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("task-card-question")
        // The field comes back open, with what was in it, whenever this view
        // is made again: the draft is the store's.
        .onAppear {
            typed = draft.read(offer.question)
            if !typed.isEmpty { writing = true }
        }
        .onChange(of: typed) { _, text in draft.write(offer.question, text) }
    }

    @ViewBuilder private var freeAnswer: some View {
        if writing {
            HStack(spacing: 8) {
                TextField("Your answer", text: $typed)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { sendTyped() }
                Button("Send") { sendTyped() }
                    .disabled(typed.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        } else {
            Button("Answer…") { writing = true }
        }
    }

    private func sendTyped() {
        let text = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        send(text)
    }

    private func send(_ answer: String) {
        guard sending == nil else { return }
        sending = answer
        failed = false
        Task {
            let written = await onAnswer(answer)
            sending = nil
            failed = !written
            if written {
                typed = ""
                writing = false
            }
        }
    }
}

/// One note in the record, drawn by its kind (`TaskNoteStyle`): an icon and
/// label in the kind's tint, the byline and time, then the text. A decision
/// is a card with its rejected options beneath; an answer that follows a
/// question sits under it on a rule; the store's own entries are one muted line.
private struct TaskNoteView: View {
    let note: TaskNoteRow
    let pairedWithQuestion: Bool

    private var style: TaskNoteStyle { .of(note.kind) }

    var body: some View {
        switch style.weight {
        case .quiet: quiet
        case .prominent: card
        case .standard: standard
        }
    }

    private var header: some View {
        HStack(spacing: 6) {
            Image(systemName: style.symbol)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .regular))
                .foregroundStyle(style.tint.color)
                .frame(width: 16)
                .accessibilityHidden(true)
            Text(style.label)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold))
                .foregroundStyle(style.tint.color)
            Text(note.byline)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
            // The board's own abbreviated words ("4m ago"), as the card's
            // line says it, with the exact time on hover.
            BoardTick { now in
                Text(TaskRow.ago(now.timeIntervalSince(note.at)))
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
                    .help(note.at.formatted(date: .abbreviated, time: .standard))
            }
            if note.supersedes != nil {
                Text("Replaces an earlier entry")
                    .font(.system(size: WorkspaceStyle.PaneText.minimum))
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var standard: some View {
        VStack(alignment: .leading, spacing: 3) {
            header
            Text(note.displayBody)
                .font(.system(size: WorkspaceStyle.PaneText.body))
                .textSelection(.enabled)
                .padding(.leading, 22)
        }
        .padding(.leading, pairedWithQuestion ? 14 : 0)
        .overlay(alignment: .leading) {
            if pairedWithQuestion {
                Capsule().fill(style.tint.color.opacity(0.5)).frame(width: 2)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var card: some View {
        let decision = TaskNoteStyle.decision(note)
        return VStack(alignment: .leading, spacing: 5) {
            header
            Text(decision.chosen)
                .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                .textSelection(.enabled)
                .padding(.leading, 22)
            if let rejected = decision.rejected {
                Text("Rejected: \(rejected)")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .padding(.leading, 22)
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(style.tint.color.opacity(0.08)))
    }

    /// "Backlog → In Progress", or the bare "Created": history, not somebody's
    /// word, so no paragraph of its own. A created note's body is the title
    /// again, so it isn't repeated.
    private var quiet: some View {
        HStack(spacing: 6) {
            Image(systemName: style.symbol)
                .frame(width: 16)
                .accessibilityHidden(true)
            Text(note.kind == .created ? style.label : "\(style.label): \(note.displayBody)")
            BoardTick { now in Text(TaskRow.ago(now.timeIntervalSince(note.at))) }
        }
        .font(.system(size: WorkspaceStyle.PaneText.secondary))
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
