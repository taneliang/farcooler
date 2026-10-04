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
// all. The orchestrator owns the task list (ov-184): the only task write this
// app makes is an answer to an agent's question, and
// `TaskManagementRemovedTests` scans every view for any other.

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
    /// Each open task's start and workers by key (`TaskStarts`); empty from
    /// a runner older than ov-212.
    @Published private(set) var starts: [String: TaskStart] = [:]
    @Published private(set) var detail: TaskDetailModel = .empty
    /// The card `detail` and `question` were read for, once they've been
    /// read: nil from `open` until the read lands. They are single slots,
    /// and a column that has just switched to another task renders once
    /// before `open` blanks them: drawn through `detail(for:)` and
    /// `question(for:)`, that frame is never the last task's (ov-65 O2).
    private var readID: String?
    /// The last few cards' records and questions, by task id, as they were
    /// when another card was opened: what a task leaving the window draws as
    /// it fades out under the next (ov-85), rather than going blank. Each is
    /// only ever drawn for its own task.
    private var recent: [(id: String, detail: TaskDetailModel, question: TaskQuestion?)] = []
    /// How many cards `recent` keeps.
    static let recentCount = 3

    /// `id`'s record: the one read for it, or, while it's leaving or until a
    /// read for it lands, the one it last showed. Empty for a task never read.
    func detail(for id: String) -> TaskDetailModel {
        if readID == id { return detail }
        return recent.first { $0.id == id }?.detail ?? .empty
    }

    /// `id`'s question, as `detail(for:)` finds its record. Nil for any
    /// other task's.
    func question(for id: String) -> TaskQuestion? {
        if readID == id { return question }
        return recent.first { $0.id == id }?.question
    }

    /// What agents spent on each task opened, by id (ov-195): read when it
    /// opens, and kept so going back to one shows it at once.
    @Published private(set) var usage: [String: TaskUsageState] = [:]

    /// `id`'s Usage section: what was read for it, or loading until then.
    func usage(for id: String) -> TaskUsageState { usage[id] ?? .loading }

    /// The card that is open, or nil for the board alone.
    @Published var opened: TaskRow?
    /// What choosing a card does: the window opens it in the workspace's
    /// task column. Nil opens it here, which is all a test needs.
    var onChoose: ((TaskRow) -> Void)?

    /// What ↑ and ↓ in the list do: the window opens the row's task beside
    /// the board, and never closes it. Nil opens it here.
    var onGlance: ((TaskRow) -> Void)?

    /// A card was clicked: open it where the window puts tasks.
    func choose(_ row: TaskRow) {
        if let onChoose { onChoose(row) } else { Task { await open(row) } }
    }

    /// A card stepped to from the keyboard: open it beside the board.
    func glance(_ row: TaskRow) {
        if let onGlance { onGlance(row) } else { Task { await open(row) } }
    }
    // MARK: - Unread (ov-104)

    /// What this person has read on this board, on this device: what the
    /// Unread section lists and the Done rule keeps (ov-103). Loaded with
    /// the store, and written as tickets are opened.
    @Published private(set) var reads: BoardReads
    /// Where `reads` is kept: this Mac's defaults, until a runner keeps it.
    let readStore: BoardReadStore
    /// Notes read for the summary, by task id.
    @Published private(set) var summaryNotes: [String: [TaskNoteRow]] = [:]
    private var noteCache: [String: (updatedAt: Date?, notes: [TaskNoteRow])] = [:]

    /// `row` was opened: everything on it so far is read, its notes up to
    /// the newest one read (`latest`).
    func markRead(_ row: TaskRow, latest: Date? = nil, now: Date = Date()) {
        var next = reads
        next.open(row, latest: latest, now: now)
        guard next != reads else { return }
        reads = next
        readStore.save(reads, host: hostKey, workspace: workspace.id)
    }

    /// The task selected, and the reads it was selected under (ov-177):
    /// Unread lists it by those (`BoardSummary.make(…held:)`), so opening it,
    /// or Mark All as Read, leaves its lines where they are until the
    /// selection moves on.
    @Published private(set) var held: HeldRead?

    /// `taskID` is selected now, or nothing is: hold the reads as they are
    /// before opening it reads it, and let go of the last one's.
    func hold(_ taskID: String?) {
        guard held?.taskID != taskID else { return }
        held = taskID.map { HeldRead(taskID: $0, reads: reads) }
    }

    /// Mark All as Read: everything on the board so far, once the person
    /// said yes (`MarkReadConfirmation`, ov-210).
    func markAllRead(_ granted: MarkReadGrant, now: Date = Date()) {
        reads.markAllRead(rows: board.rows, now: now)
        readStore.save(reads, host: hostKey, workspace: workspace.id)
    }

    /// Read the records of the tasks that moved since they were read, so
    /// Unread can list their notes. One `task show` each, remembered until
    /// the task's `updatedAt` moves.
    func readSummaryNotes(reads: BoardReads) async {
        let picked = noteCandidates(reads: reads)
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

    /// The tasks whose notes carry `query`, by id: the History page's note
    /// search, answered by the runner (`task search`). Empty for a runner
    /// that can't, or for nothing found.
    func noteHits(_ query: String) async -> Set<String> {
        let (data, _) = await client.taskSearch(query: query, repository: repositoryID)
        guard let data, let hits = try? JSONDecoder().decode(NoteHits.self, from: data) else { return [] }
        let byKey = Dictionary(board.rows.map { ($0.key, $0.id) }, uniquingKeysWith: { a, _ in a })
        return Set(hits.hits.compactMap { $0.key.flatMap { byKey[$0] } })
    }

    private struct NoteHits: Decodable {
        struct Hit: Decodable { var key: String? }
        var hits: [Hit]
    }

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

    init(client: DaemonClient, workspace: WorkspaceSummary, readStore: BoardReadStore = DefaultsBoardReads()) {
        self.client = client
        self.workspace = workspace
        self.seenGeneration = client.boardGeneration(for: workspace)
        self.readStore = readStore
        self.reads = readStore.load(
            host: client.target.isEmpty ? "local" : client.target, workspace: workspace.id, now: Date())
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
        // When each task starts and who works it (ov-212, ov-213), from the
        // same read: the title bar's queue and its activity panel.
        let reading = TaskStarts.decode(data)
        if reading != starts { starts = reading }
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
            // What the card going away showed, kept for it under its own id.
            if let readID {
                recent.removeAll { $0.id == readID }
                recent.insert((readID, detail, question), at: 0)
                recent = Array(recent.prefix(Self.recentCount))
            }
            opened = row
            readID = nil
            detail = .empty
            question = nil
        }
        await read(row)
        await readUsage(row)
    }

    /// Read what agents spent on `row`. A runner too old to record it says
    /// it needs an update; a read that fails keeps what was shown before,
    /// and with nothing shown, offers Try Again.
    func readUsage(_ row: TaskRow) async {
        let can = client.daemonBuild?.can("agent_usage")
        if can == false {
            usage[row.id] = .needsUpdate
            return
        }
        let (data, message) = await client.taskUsage(key: row.key, repository: repositoryID)
        // The CLI's own refusal for a runner without `agent_usage`
        // (`task_usage.rs`, NO_USAGE), for a build not yet read.
        if data == nil, message?.contains("older than usage reports") == true {
            usage[row.id] = .needsUpdate
            return
        }
        let read = data.flatMap { try? TaskUsage.decode($0) }
        if read == nil, case .loaded? = usage[row.id] { return }
        usage[row.id] = TaskUsageState.after(read: read, runnerCan: can)
    }

    /// Try Again on a Usage section whose read failed.
    func retryUsage(_ row: TaskRow) {
        usage[row.id] = .loading
        Task { await readUsage(row) }
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
        // Open on screen, and read: its unread items go (ov-104), as do
        // notes written while it stays open.
        markRead(row, latest: read.notes.map(\.at).max())
        let asked = TaskQuestion.open(in: data)
        // Assigned only when it changed, so an unchanged question keeps its
        // view, and the field in it.
        if asked != question { question = asked }
        // The blocks arrive as ids; the board is what turns them into keys.
        opened = row.with(blocks: board.resolvingBlocks(read.blocks))
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

    /// Whether this board offers its one write: a question's Answer buttons.
    /// `TaskBoardWrites.offered`'s rule over this runner's build.
    var offersWrites: Bool { TaskBoardWrites.offered(by: client.daemonBuild) }

    /// Whether `id`'s card offers its question's answers: only from a read
    /// made for it since it was opened. A record kept from before (`recent`:
    /// a task leaving under the next, or one reopened until its read lands)
    /// is drawn read-only, since its question may have been answered since.
    func canAnswer(_ id: String) -> Bool { offersWrites && readID == id }

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
            counts[title, default: 0] > 1 ? "\(title) (\(pane.terminal.short))" : title  // not a count: a twin pane told apart by its id
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

    /// The pane `row`'s subagents live in, or nil (`TaskWorkers.orchestrator`).
    func orchestrator(for row: TaskRow) -> BoardPane? {
        runnerRecordsTasks ? TaskWorkers.orchestrator(of: row, in: worktrees) : nil
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
/// A question's Answer buttons are a Control-scope write (`task.note`), the
/// only task write this app makes: the orchestrator owns the rest of the task
/// list (ov-184). A connection granted only Read sees the board without them,
/// which is the rule Needs You follows too (spec §2.5).
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
    /// Go to a pane working a task. The window's, because only the window can
    /// change what is selected.
    let onGoTo: (BoardPane) -> Void
    /// Where the collapsed sections are kept. The app's own defaults,
    /// except in a test.
    let defaults: UserDefaults
    /// The task open beside the board, drawn selected (ov-85).
    let selected: String?
    /// Bumped when the window gives the list the keyboard: ⌥⌘2, a row
    /// chosen, or what's opened closed.
    let focusRequest: Int
    /// The list took the keyboard on its own, from a click: the window's
    /// terminals let go of it.
    let onKeyboard: () -> Void
    /// Return with a task open: the keyboard into it, as ⌥⌘3.
    let onEnter: () -> Void
    /// The window has given the board the keyboard (⌥⌘2, a row chosen, a
    /// close), and no terminal has taken it back since: what ↑ and ↓ need,
    /// and what draws the selected row in the accent.
    let hasKeyboard: Bool
    /// The workspace's worktrees (ov-86): each task's, named on its row,
    /// and the loose ones, in the Worktrees section under the tasks. Asked
    /// of the board as it is now, here, where the board is observed: worked
    /// out by the window, it stayed as it was before the first read.
    let worktreesOf: (TaskBoardModel) -> BoardWorktrees
    /// The orchestrator's row, at the top (ov-92): nil where the workspace
    /// can't have one.
    let orchestrator: NavigatorOrchestrator?
    /// The row the window's selection lights (`Navigator.current`).
    let current: NavigatorItem?
    /// ↑ or ↓ onto a row: the window selects it, keeping the keyboard
    /// here. Nil opens a task here, which is all a test needs.
    let onStep: ((NavigatorItem) -> Void)?
    /// A status's History page, opened in the main area (ov-103).
    let onHistory: (TaskStatus) -> Void
    /// Bumped by ⌘F: the filter field takes the keyboard.
    let filterRequest: Int
    /// Ask the Orchestrator, on each row's menu.
    let ask: AskOrchestrator.Action

    /// The list's collapsed sections: read from `defaults` in `init`, and
    /// again when the view is handed another board.
    @State private var collapsed: Set<TaskStatus>
    /// The navigator's sections closed on this Mac: Orchestrator, Tasks,
    /// Worktrees. Their rows leave ↑ and ↓'s walk while closed.
    @State private var closedSections: Set<String>
    /// The long sections showing all of their tasks, not just ten: kept
    /// here, not in each section, since ↑ and ↓ walk the rows they show.
    @State private var showingMore: Set<TaskStatus> = []
    /// The navigator's filter (⌘F, ov-103): every section narrowed to the
    /// tasks whose key or title carries what's typed.
    @State private var filter = ""
    @FocusState private var filterFocused: Bool
    /// The Unread line the selection was chosen at, while its task is the
    /// one selected: lit there, in place, and where ↑ and ↓ go on from
    /// (ov-177).
    @State private var unreadLine: String?
    /// Unread closed: kept here rather than in the strip, since ↑ and ↓ walk
    /// its lines only while it's open.
    @State private var unreadCollapsed: Bool
    /// Where a task's row is matched as it moves between statuses.
    @Namespace private var rowSpace
    /// The list has the keyboard: ↑ and ↓ glance through the tasks, Return
    /// opens one.
    @FocusState private var listFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// The window this board is in, and the monitor that hears its arrows.
    @State private var windowBox = WindowBox()
    @State private var arrowMonitor: Any?
    /// What the monitor reads, kept in state as they change: it was made
    /// with this view as it was then, so `selected` and the focus it
    /// captured would stay as they were (live, ov-85: it never moved).
    @State private var heard = Heard()
    final class Heard {
        var keyed = false
        var selected: NavigatorItem?
        /// The row the last ↑ or ↓ went to, until the window's selection
        /// catches up: key repeats come faster than it redraws, and each
        /// must step on from the last, not from where the window still says
        /// it is.
        var stepped: NavigatorItem?
        /// The rows shown, top to bottom, every section's
        /// (`Navigator.items`), and the board they're on.
        var items: [NavigatorItem] = []
        weak var store: TaskBoardStore?
        var onStep: ((NavigatorItem) -> Void)?
        /// An Unread line stepped to, or nil for any other row: the view
        /// lights it there.
        var onLine: ((String?) -> Void)?

        /// ↑ or ↓: the row above or below the one selected (or the last
        /// one stepped to), across the sections, selected in its place.
        @MainActor
        func step(_ by: Int) -> KeyPress.Result {
            let from = stepped ?? selected
            guard let next = Navigator.step(from: from, by: by, in: items), next != from
            else { return from == nil ? .ignored : .handled }
            stepped = next
            if case .unread(let line) = next { onLine?(line) } else { onLine?(nil) }
            // An Unread line opens its task, held where it is (ov-177).
            if let id = next.taskID { store?.hold(id) }
            if let onStep {
                onStep(next.taskID.map(NavigatorItem.task) ?? next)
            } else if let id = next.taskID, let store, let row = BoardKeys.row(id, in: store.board) {
                store.glance(row)
            }
            return .handled
        }
    }

    init(
        store: TaskBoardStore, client: DaemonClient, agents: BoardAgents,
        onGoTo: @escaping (BoardPane) -> Void, defaults: UserDefaults = .standard,
        selected: String? = nil, focusRequest: Int = 0, onKeyboard: @escaping () -> Void = {},
        onEnter: @escaping () -> Void = {}, hasKeyboard: Bool = false,
        worktrees: @escaping (TaskBoardModel) -> BoardWorktrees = { _ in .none },
        orchestrator: NavigatorOrchestrator? = nil, current: NavigatorItem? = nil,
        onStep: ((NavigatorItem) -> Void)? = nil, onHistory: @escaping (TaskStatus) -> Void = { _ in },
        filterRequest: Int = 0, ask: AskOrchestrator.Action = .unavailable
    ) {
        self.store = store
        self.client = client
        self.agents = agents
        self.onGoTo = onGoTo
        self.defaults = defaults
        self.selected = selected
        self.focusRequest = focusRequest
        self.onKeyboard = onKeyboard
        self.onEnter = onEnter
        self.hasKeyboard = hasKeyboard
        self.worktreesOf = worktrees
        self.orchestrator = orchestrator
        self.current = current ?? selected.map(NavigatorItem.task)
        self.onStep = onStep
        self.onHistory = onHistory
        self.filterRequest = filterRequest
        self.ask = ask
        _collapsed = State(
            initialValue: BoardForm.collapsed(
                host: store.hostKey, workspace: store.workspace.id, from: defaults))
        _closedSections = State(initialValue: Self.closedSections(store, defaults))
        _unreadCollapsed = State(initialValue: defaults.bool(forKey: BoardSummaryStrip.collapsedKey(store)))
    }

    var body: some View {
        VStack(spacing: 0) {
            topBand
            list
        }
        // One key column for the whole board, so every title starts at one x.
        .environment(\.taskKeyWidth, TaskKeyColumn.width(for: store.board.rows.map(\.key)))
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
        .onChange(of: remembered) { _, key in
            showingMore = []
            filter = ""
            unreadLine = nil
            collapsed = BoardForm.collapsed(host: key.host, workspace: key.workspace, from: defaults)
            closedSections = Self.closedSections(store, defaults)
            unreadCollapsed = defaults.bool(forKey: BoardSummaryStrip.collapsedKey(store))
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
        VStack { content() }.frame(maxWidth: .infinity, minHeight: 6 * ColumnGrid.rhythm)
    }

    // MARK: - The top band

    /// The navigator's top band (ov-214): the filter, then the board's own
    /// state at the trailing edge. It took the place of a header row that
    /// named the board ("Main", which the title bar's switcher names),
    /// counted what's waiting (which the title bar's status area counts) and
    /// offered New Task…, which went with ov-184: the orchestrator owns the
    /// list, and the title bar's field asks it (⌘K). Refresh is ⌘R.
    private var topBand: some View {
        HStack(spacing: SidebarGrid.gap / 2) {
            filterField
            if store.reading {
                ProgressView().controlSize(.small)
                    .help("Reading the board")
            }
            // Beside the board rather than over it: a failed re-read
            // leaves the last good board on screen.
            if store.hasRead, let trouble = store.trouble {
                Image(systemName: "exclamationmark.triangle")
                    .foregroundStyle(.secondary)
                    .frame(width: SidebarGrid.control, height: SidebarGrid.control)
                    .help(trouble)
                    .accessibilityLabel(trouble)
            }
        }
        // On the list's grid (ov-177): the filter's box from the grid's
        // edge, where a row's selection runs, to as far in from the
        // trailing edge, and a rhythm over it; the list's own top inset is
        // the rhythm under it.
        .padding(.horizontal, NavigatorGrid.edge)
        .padding(.top, NavigatorRhythm.band)
        .onChange(of: filterRequest) { _, _ in filterFocused = true }
    }

    // MARK: - The list

    /// The filter field atop the navigator (⌘F): Esc clears it, and on an
    /// empty field gives the list the keyboard back.
    private var filterField: some View {
        NavigatorFilterField(text: $filter, focused: $filterFocused) {
            filterFocused = false
            listFocused = true
        }
    }

    private var filtering: Bool { !BoardFilter.isEmpty(filter) }

    /// What the navigator draws for the filter as it is (`NavigatorFiltering`).
    private func plan(worktrees: BoardWorktrees, shown: TaskBoardModel) -> NavigatorFiltering {
        NavigatorFiltering.make(
            filter: filter, board: shown, hasOrchestrator: orchestrator != nil, agent: orchestrator?.agent,
            unreadMatches: store.hasRead
                && !BoardSummaryStrip.summary(store: store, reads: store.reads, filter: filter).isEmpty,
            worktrees: worktrees)
    }

    /// The row lit: the window's, or the Unread line its task was chosen
    /// at (`Navigator.place`).
    private var place: NavigatorItem? { Navigator.place(current, line: unreadLine) }

    /// The Unread line lit, if the selection is lit there.
    private var litLine: String? {
        if case .unread(let line)? = place { return line }
        return nil
    }

    /// An Unread line clicked: its task opens, lit on the line. A line of
    /// the task already selected only moves the light there; clicked again,
    /// it closes the task, as a row does.
    private func chooseLine(_ line: String) {
        let id = BoardSummaryStrip.task(ofLine: line)
        listFocused = true
        guard let row = store.board.rows.first(where: { $0.id == id }) else { return }
        let moving = selected == id && litLine != line
        unreadLine = line
        if !moving { store.choose(row) }
    }

    /// A task's row clicked: as `chooseLine`, from its status.
    private func chooseRow(_ row: TaskRow) {
        listFocused = true
        let moving = selected == row.id && litLine != nil
        unreadLine = nil
        if !moving { store.choose(row) }
    }

    /// The navigator's sections (ov-92), a divider apart: the orchestrator's
    /// row, then Tasks, the tasks by status under Unread, and Worktrees, the
    /// loose worktrees. The orchestrator's is a row and not a section
    /// (ov-177): there's only ever one, and the row says what it is.
    ///
    /// While the filter narrows it, only what matches is drawn, and with
    /// nothing matching, one "No Results" (`NavigatorFiltering`); and the
    /// list jumps to what matches rather than moving there
    /// (`unanimatedWhenFiltering`).
    private var list: some View {
        let shown = BoardFilter.narrowed(store.board, filter)
        let plan = self.plan(worktrees: worktreesOf(store.board), shown: shown)
        let worktrees = plan.worktrees
        // The repository's own terminals, between Tasks and Worktrees
        // (ov-178): the filter narrows them as it does the rest.
        let terminals = worktrees.terminals.narrowed(by: filter)
        let inProgress = store.board.columns.first { $0.status == .inProgress }?.rows.count ?? 0
        let unreadable = !shown.unreadable.isEmpty
        // The sections' slots touch: each one's room over it is its own
        // (`NavigatorRhythm`, ov-243), a rule's half the section gap on
        // either side, and a section that follows another with no rule
        // between them the whole section gap.
        let showsTasks = plan.showsTasks(unreadable: unreadable)
        let ruleUnderOrchestrator = orchestrator != nil && plan.showsOrchestrator
            && (showsTasks || terminals.isShown || plan.showsWorktrees)
        let ruleUnderTasks = showsTasks && (terminals.isShown || plan.showsWorktrees)
        return ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if plan.isEmpty(unreadable: unreadable) && !terminals.isShown {
                        NavigatorNoResults(filter: filter)
                    }
                    if let orchestrator, plan.showsOrchestrator {
                        OrchestratorRowView(
                            model: orchestrator, inProgress: inProgress, selected: place == .orchestrator,
                            keyed: hasKeyboard)
                        .id(NavigatorItem.orchestrator)
                        .padding(.horizontal, NavigatorGrid.edge)
                        if ruleUnderOrchestrator {
                            Divider().probed("navigator-divider")
                                .padding(.vertical, NavigatorRhythm.rule)
                        }
                    }
                    if showsTasks {
                        section("Tasks", id: "tasks") {
                            tasks(plan, shown: shown, worktrees: worktrees)
                        }
                    }
                    // One rule under the tasks, before whatever follows
                    // them; Terminals and Worktrees, the two lists that
                    // aren't tasks, are set apart by space (ov-216's
                    // `Spacing.section`), not by another line.
                    if ruleUnderTasks {
                        Divider().probed("navigator-divider")
                            .padding(.vertical, NavigatorRhythm.rule)
                    }
                    if terminals.isShown {
                        // No count when it only offers New Terminal: "0"
                        // would read as something to look at.
                        section(
                            "Terminals", id: "terminals",
                            count: terminals.terminals.isEmpty ? nil : terminals.terminals.count
                        ) {
                            ProjectTerminalsSection(terminals: terminals, keyed: hasKeyboard)
                        }
                    }
                    if plan.showsWorktrees {
                        section("Worktrees", id: "worktrees", count: worktrees.shown.count) {
                            BoardWorktreesSection(worktrees: worktrees, keyed: hasKeyboard)
                        }
                        .padding(.top, terminals.isShown ? NavigatorRhythm.section : 0)
                    }
                }
                .padding(.vertical, NavigatorRhythm.band)
                .unanimatedWhenFiltering(filter)
            }
            // The row selected stays in sight as ↑ and ↓ step past the edge,
            // scrolled by as little as that takes.
            .onChange(of: place) { _, item in
                guard let item, hasKeyboard else { return }
                withAnimation(WorkspaceMotion.spring) {
                    switch item {
                    case .task(let id): proxy.scrollTo(id)
                    default: proxy.scrollTo(item)
                    }
                }
            }
        }
        .focusable()
        .focused($listFocused)
        .focusEffectDisabled()
        // ↑ and ↓, pressed or held: heard from the window's key events,
        // since SwiftUI drops a held key's repeats once the view they started
        // in has redrawn, which every step does (live, ov-85: twelve repeats
        // moved it one row). Each step retargets the same spring.
        .background(WindowReader(box: windowBox))
        .onAppear { listenForArrows() }
        .onDisappear {
            if let arrowMonitor { NSEvent.removeMonitor(arrowMonitor) }
            arrowMonitor = nil
        }
        .onKeyPress(.return) {
            // A row selected, the orchestrator included: into it, as ⌥⌘3.
            // Nothing selected: the first row.
            if current != nil {
                onEnter()
                return .handled
            }
            return heard.step(1)
        }
        .onChange(of: focusRequest) { _, _ in listFocused = true }
        // The window caught up with the steps taken.
        .onChange(of: place, initial: true) { _, now in
            heard.stepped = nil
            heard.selected = now
        }
        // The selection moved off the line's task: the line is let go of,
        // so coming back to the task from elsewhere lights its row.
        .onChange(of: current) { _, now in
            if let line = unreadLine, now?.taskID != BoardSummaryStrip.task(ofLine: line) { unreadLine = nil }
        }
        // What's selected holds its Unread lines in place, however it was
        // chosen: here, from a notice, from the palette (ov-177).
        .onChange(of: selected, initial: true) { _, id in store.hold(id) }
        .onChange(of: listFocused) { _, focused in if focused { onKeyboard() } }
        .onChange(of: hasKeyboard, initial: true) { _, keyed in heard.keyed = keyed }
        .onChange(of: items(worktrees: worktreesOf(store.board)), initial: true) { _, items in heard.items = items }
        .onChange(of: ObjectIdentifier(store), initial: true) { _, _ in
            heard.store = store
            store.hold(selected)
        }
        // On every update, so a later capture in it can never go stale.
        .background {
            let _ = heard.onStep = onStep
            let _ = heard.onLine = { unreadLine = $0 }
            Color.clear
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-list")
    }

    /// The Tasks section's content: Unread, then each status.
    @ViewBuilder
    private func tasks(_ plan: NavigatorFiltering, shown: TaskBoardModel, worktrees: BoardWorktrees) -> some View {
        // Its groups, Unread and each status, a group gap apart.
        VStack(alignment: .leading, spacing: NavigatorRhythm.group) {
            if store.hasRead, plan.showsUnread {
                // Edge to edge, its own inset at column A.
                BoardSummaryStrip(
                    store: store, defaults: defaults, filter: filter, selectedLine: litLine, keyed: hasKeyboard,
                    collapsed: $unreadCollapsed, onChooseLine: { chooseLine($0) }
                )
                .id(ObjectIdentifier(store))
                .padding(.horizontal, -NavigatorGrid.edge)
            }
            if !store.hasRead && store.reading {
                centered { ProgressView() }
            } else if let trouble = store.trouble, !store.hasRead {
                centered {
                    VStack(spacing: NavigatorRhythm.group) {
                        Text(trouble)
                        Button("Try Again") { Task { await store.reload() } }
                    }
                }
            } else {
                ForEach(plan.sections) { section in
                    TaskListSection(
                        section: section,
                        // Filtering opens every section with a match.
                        expanded: BoardForm.isExpanded(section, collapsed: filtering ? [] : collapsed),
                        onToggle: { toggle(section.status) },
                        store: store, agents: agents, onGoTo: onGoTo,
                        selected: selected, lit: litLine == nil ? selected : nil, keyed: hasKeyboard,
                        worktrees: worktrees,
                        showingMore: showingMore.contains(section.status),
                        onShowMore: {
                            withAnimation(BoardMotion.list(reduceMotion: reduceMotion)) {
                                if showingMore.contains(section.status) {
                                    showingMore.remove(section.status)
                                } else {
                                    showingMore.insert(section.status)
                                }
                            }
                        },
                        filtering: filtering,
                        onHistory: onHistory,
                        onChoose: { chooseRow($0) },
                        ask: ask,
                        rows: rowSpace)
                }
                if !shown.unreadable.isEmpty {
                    UnreadableColumnView(rows: shown.unreadable)
                }
            }
        }
    }

    /// The rows the navigator shows, top to bottom: what ↑ and ↓ walk.
    /// A closed section's rows aren't shown, so they aren't walked, nor
    /// are what the filter leaves out.
    private func items(worktrees all: BoardWorktrees) -> [NavigatorItem] {
        let shown = BoardFilter.narrowed(store.board, filter)
        let plan = self.plan(worktrees: all, shown: shown)
        let tasksOpen = !closedSections.contains("tasks")
        let unread = tasksOpen && store.hasRead && plan.showsUnread && !unreadCollapsed
            ? BoardSummaryStrip.lines(BoardSummaryStrip.summary(store: store, reads: store.reads, filter: filter))
            : []
        return Navigator.items(
            orchestrator: orchestrator != nil && plan.showsOrchestrator,
            unread: unread,
            tasks: tasksOpen
                ? BoardKeys.rows(
                    shown, collapsed: collapsed, reads: store.reads, keeping: selected, showingMore: showingMore,
                    filtering: filtering, now: Date())
                : [],
            // The Terminals section's rows, as `list` draws them: none when
            // it's closed, and what the filter leaves otherwise (ov-234).
            terminals: closedSections.contains("terminals")
                ? [] : plan.worktrees.terminals.narrowed(by: filter).terminals.map(\.id),
            worktrees: closedSections.contains("worktrees") || !plan.showsWorktrees
                ? [] : BoardWorktreesSection.rows(plan.worktrees).map(\.id))
    }

    private static func closedSections(_ store: TaskBoardStore, _ defaults: UserDefaults) -> Set<String> {
        Set(navigatorSections.filter { defaults.bool(forKey: closedKey($0, store: store)) })
    }

    /// Where a navigator section's being closed is kept, per board.
    static func closedKey(_ id: String, store: TaskBoardStore) -> String {
        "navigator.closed.\(store.hostKey).\(store.workspace.id).\(id)"
    }

    /// The navigator's collapsible sections. Not the orchestrator's row
    /// (ov-177), whose closed state, kept from before, is no longer read.
    static let navigatorSections = ["tasks", "terminals", "worktrees"]

    /// A navigator section (ov-92): the one collapsible section, in the
    /// navigator's style, its open state kept per board on this Mac.
    private func section<Content: View>(
        _ title: String, id: String, count: Int? = nil, @ViewBuilder _ content: @escaping () -> Content
    ) -> some View {
        CollapsibleSection(
            title, id: id, style: .navigator,
            isExpanded: Binding(
                get: { !closedSections.contains(id) },
                set: { open in
                    if open { closedSections.remove(id) } else { closedSections.insert(id) }
                    defaults.set(!open, forKey: Self.closedKey(id, store: store))
                }),
            count: count, content: content)
        .padding(.horizontal, NavigatorGrid.edge)
    }

    /// Listen for ↑ and ↓, repeats included, in this board's window while
    /// the list has the keyboard.
    private func listenForArrows() {
        guard arrowMonitor == nil else { return }
        // The box and the window's box, not this view: the monitor outlives
        // the copy of the view it was made in.
        let heard = heard
        let box = windowBox
        arrowMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            guard heard.keyed, let window = event.window, window === box.window, window.attachedSheet == nil,
                !EscapeBack.keepsEscape(window.firstResponder),
                let by = BoardKeys.arrow(keyCode: event.keyCode, modifiers: event.modifierFlags)
            else { return event }
            return heard.step(by) == .handled ? nil : event
        }
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

/// One task in the navigator (ov-104): a compact row, its key and title,
/// and under the title one quiet line saying what the status header doesn't
/// (`TaskRowMeta`): what it waits on, who's on it, how much of it holds.
/// The way to its agent and its worktree's menu are in its context menu.
///
/// Internal rather than private for `BoardCardTickTests`, which draws it.
struct TaskListRow: View {
    let row: TaskRow
    let prominent: Bool
    @ObservedObject var store: TaskBoardStore
    let live: [BoardPane]
    let presence: TaskAgentPresence
    let onGoTo: (BoardPane) -> Void
    /// The pane its subagents live in (ov-213), when it has some.
    var orchestrator: BoardPane?
    /// Whether the runner can be believed about its agents right now.
    var speaksOfAgents = true
    /// Open beside the board (ov-85).
    var selected = false
    /// The list has the keyboard: selected reads in the accent.
    var keyed = false
    /// Its worktree (ov-86), whose menu is on the row's.
    var worktree: Worktree?
    var worktreeMenu: [WorktreeMenu.Item] = []
    var performOnWorktree: (WorktreeMenu.Item) -> Void = { _ in }
    var onChoose: (() -> Void)?
    /// Ask the Orchestrator about it, in the context menu.
    var ask: AskOrchestrator.Action = .unavailable

    var body: some View {
        CompactTaskRow(key: row.key, title: row.title, emphasized: prominent, selected: selected, keyed: keyed) {
            TaskRowMetaView(
                row: row, agent: TaskRowMeta.agent(live: live, presence: presence),
                speaksOfAgents: speaksOfAgents)
        }
        .contentShape(Rectangle())
        .onTapGesture { if let onChoose { onChoose() } else { store.choose(row) } }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .contextMenu {
            TaskRowMenu(row: row, live: live, orchestrator: orchestrator, onGoTo: onGoTo, ask: ask)
            if let worktree, !worktreeMenu.isEmpty {
                Divider()
                Menu("Worktree \(worktree.task)") {
                    WorktreeMenuItems(items: worktreeMenu, perform: performOnWorktree)
                }
            }
        }
        .identified("board-row-\(row.key)")
    }
}

/// A card's context menu: the way to its agent, and Ask the Orchestrator.
private struct TaskRowMenu: View {
    let row: TaskRow
    let live: [BoardPane]
    var orchestrator: BoardPane?
    let onGoTo: (BoardPane) -> Void
    let ask: AskOrchestrator.Action

    var body: some View {
        GoToAgentItems(live: live, onGoTo: onGoTo)
        // Where its subagents live, when no pane of its own is on it.
        if live.isEmpty, let orchestrator {
            Button("Go to Orchestrator") { onGoTo(orchestrator) }
        }
        if !live.isEmpty || orchestrator != nil { Divider() }
        AskOrchestratorButton(row: row, action: ask)
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
            VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                Text("Not On This Version").font(WorkspaceStyle.sectionTitle)
                    .gridMark("unreadable", .text)
                Text("This runner uses states this Far Cooler doesn’t have yet.")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.leading, NavigatorGrid.textInset)
            ForEach(rows) { row in
                VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
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
                .padding(.horizontal, NavigatorGrid.textInset)
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
    /// What its agents spent (ov-195). Nil draws no Usage section.
    var usage: TaskUsageState? = nil
    /// The pane its subagents live in (ov-213), for the Subagents section.
    var orchestrator: BoardPane?
    var onGoTo: (BoardPane) -> Void = { _ in }
    /// Whether the runner can be believed about its agents right now.
    var speaksOfAgents = true
    /// Try Again, on a Usage section whose read failed.
    var onRetryUsage: () -> Void = {}
    /// Send an answer; true when it was written.
    let onAnswer: (String) async -> Bool
    /// Where Answer…'s unsent text is kept: the store, so it outlives this
    /// view, which is redrawn whenever the card's record is read again.
    var draft: Draft = .none
    /// The task keys its lines link (ov-196), and where they go.
    @Environment(\.taskKeyLinker) private var linker

    /// Reads and writes an unsent answer, by question.
    struct Draft {
        var read: (TaskQuestion) -> String
        var write: (TaskQuestion, String) -> Void

        /// Nowhere: a card nobody can type into, or a test's.
        static var none: Draft { Draft(read: { _ in "" }, write: { _, _ in }) }
    }

    /// Its sections, a rhythm-multiple apart, at the reading measure
    /// (`TaskTypography`): the present, the question, then the record.
    var body: some View {
        VStack(alignment: .leading, spacing: TaskTypography.sectionGap) {
            understanding
            if let offer = Self.offer(row: row, question: question, canAnswer: canAnswer) {
                QuestionAnswers(offer: offer, onAnswer: onAnswer, draft: draft)
                    .id(offer.question.id)
            }
            if speaksOfAgents, !row.workers.isEmpty {
                TaskWorkersSection(row: row, orchestrator: orchestrator, onGoTo: onGoTo)
            }
            if let usage {
                TaskUsageView(state: usage, onRetry: onRetryUsage)
            }
            Divider()
            record
        }
        .frame(maxWidth: TaskTypography.measure, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .leading)
        // Links in task text open on the web or in mail, or a task key's
        // task here, nothing else (`Markdown.opens`), as in `MarkdownText`.
        .environment(\.openURL, Markdown.openGuard(linker))
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
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm / 2) {
            if let waiting = row.blockedSummary {
                Text(waiting)
                    .font(TaskTypography.body.weight(.medium))
                    .foregroundStyle(.orange)
                ForEach(row.blockedBy, id: \.key) { block in
                    if !block.reason.isEmpty {
                        Text("\(block.key) — \(block.reason)")
                            .font(TaskTypography.meta)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            BoardTick { now in
                if let line = row.startLine(at: now, speaksOfAgents: speaksOfAgents) {
                    Text(line)
                        .font(TaskTypography.meta)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("task-start-line")
                }
            }
            CardTimeLines(row: row, staleSize: TaskTypography.metaSize, timeSize: TaskTypography.metaSize)
        }
        if !row.intent.isEmpty {
            section("Intent") {
                MarkdownText(text: row.intent, spacing: .document)
                    .accessibilityIdentifier("task-intent")
            }
        }
        if !row.acceptance.isEmpty {
            section("Acceptance", detail: row.acceptanceProgress?.sentence) {
                VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
                    ForEach(row.acceptance) { line in
                        let linked = linker.linked(TaskProse.acceptance(line.text, met: line.met))
                        // Monochrome: a met line is ticked and struck
                        // through, quietly; one still open is plain.
                        HStack(alignment: .firstTextBaseline, spacing: ColumnGrid.rhythm) {
                            Image(systemName: line.met ? "checkmark.square" : "square")
                                .foregroundStyle(.secondary)
                                .accessibilityHidden(true)
                            Text(linked)
                                .font(TaskTypography.body)
                                .foregroundStyle(line.met ? Color.secondary : Color.primary)
                                .fixedSize(horizontal: false, vertical: true)
                                .textSelection(.enabled)
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityValue(line.met ? "Met" : "Not met")
                        // Its task links, as actions: a combined element
                        // needn't keep its text's links (ov-196).
                        .taskKeyActions(linked, linker: linker)
                    }
                }
            }
        }
        if !row.constraints.isEmpty {
            section("Constraints") {
                VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
                    ForEach(row.constraints, id: \.self) { text in
                        HStack(alignment: .firstTextBaseline, spacing: ColumnGrid.rhythm) {
                            Text("•").foregroundStyle(.secondary)
                            Text(linker.linked(TaskProse.inline(text)))
                                .fixedSize(horizontal: false, vertical: true)
                                .textSelection(.enabled)
                        }
                        .font(TaskTypography.body)
                    }
                }
            }
        }
    }

    @ViewBuilder private var record: some View {
        section("Record") {
            VStack(alignment: .leading, spacing: TaskTypography.noteGap) {
                if detail.notes.isEmpty {
                    Text("Nothing written down yet.")
                        .font(TaskTypography.body)
                        .foregroundStyle(.secondary)
                }
                let feed = TaskNoteStyle.feed(detail.notes)
                let paired = feed.paired
                ForEach(feed.notes) { note in
                    TaskNoteView(
                        note: note, pairedWithQuestion: paired.contains(note.id),
                        tint: TaskNoteStyle.tint(of: note, in: feed.notes, status: row.status))
                }
                // Never silently: an entry this build cannot name is still an
                // entry, and the record's claim is that nothing in it is lost.
                if detail.unreadableNotes > 0 {
                    Text(unreadableSentence)
                        .font(TaskTypography.meta)
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

    /// A section: its label, and beside it a quiet `detail` ("2 of 3
    /// met"), then what it heads, a rhythm below.
    @ViewBuilder private func section<Content: View>(
        _ title: String, detail: String? = nil, @ViewBuilder _ content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: TaskTypography.labelGap) {
            HStack(alignment: .firstTextBaseline, spacing: ColumnGrid.rhythm) {
                Text(title)
                    .font(TaskTypography.label)
                    .foregroundStyle(.secondary)
                    .accessibilityAddTraits(.isHeader)
                if let detail {
                    Text(detail)
                        .font(TaskTypography.meta)
                        .foregroundStyle(.tertiary)
                }
            }
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
    @Environment(\.taskKeyLinker) private var linker

    var body: some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
            Text("Question")
                .font(TaskTypography.label)
                .foregroundStyle(Color.accentColor)
            Text(linker.linked(TaskProse.inline(offer.question.body)))
                .font(TaskTypography.body.weight(.medium))
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            if sending != nil {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Answering…")
                        .font(TaskTypography.meta)
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
                    .font(TaskTypography.meta)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(1.5 * ColumnGrid.rhythm)
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

/// One note in the record, a block (ov-98): a quiet "<Kind> · byline ·
/// time" line, monochrome but for a question still waiting on the person,
/// then the body as
/// Markdown. A decision is a card with its rejected options beneath; an
/// answer that follows a question sits under it on a rule; the store's own
/// entries are the quiet line alone.
private struct TaskNoteView: View {
    let note: TaskNoteRow
    let pairedWithQuestion: Bool
    /// The label's color: secondary, or the accent for a question still
    /// waiting on the person (`TaskNoteStyle.tint(of:in:status:)`).
    var tint: TaskNoteStyle.Tint = .secondary
    @Environment(\.taskKeyLinker) private var linker

    private var style: TaskNoteStyle { .of(note.kind) }

    var body: some View {
        switch style.weight {
        case .quiet: quiet
        case .prominent: card
        case .standard: standard
        }
    }

    /// "Finding · manager · 4m ago", the kind in its tint, the rest
    /// secondary, with the exact time on hover.
    private var line: some View {
        BoardTick { now in
            let ago = TaskRow.ago(now.timeIntervalSince(note.at))
            let rest = TaskProse.noteLine(kind: "", byline: note.byline, ago: ago)
            let kind = Text(style.label).foregroundStyle(tint.color).fontWeight(.medium)
            let replaces = note.supersedes != nil ? " · Replaces an earlier entry" : ""
            Text("\(kind)\(Text(" · \(rest)\(replaces)").foregroundStyle(.secondary))")
                .font(TaskTypography.meta)
                .lineLimit(1)
                .help(note.at.formatted(date: .abbreviated, time: .standard))
        }
    }

    private var standard: some View {
        VStack(alignment: .leading, spacing: TaskTypography.noteLineGap) {
            line
            MarkdownText(text: note.displayBody, spacing: .document)
        }
        .padding(.leading, pairedWithQuestion ? 2 * ColumnGrid.rhythm : 0)
        .overlay(alignment: .leading) {
            if pairedWithQuestion {
                Capsule().fill(Color.secondary.opacity(0.5)).frame(width: 2)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var card: some View {
        let decision = TaskNoteStyle.decision(note)
        return VStack(alignment: .leading, spacing: TaskTypography.noteLineGap) {
            line
            MarkdownText(text: decision.chosen, spacing: .document)
            if let rejected = decision.rejected {
                Text(linker.linked(TaskProse.inline("Rejected: \(rejected)")))
                    .font(TaskTypography.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                    .padding(.top, ColumnGrid.rhythm / 2)
            }
        }
        .padding(1.5 * ColumnGrid.rhythm)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(Color.primary.opacity(0.05)))
    }

    /// "Status Change: Backlog → In Progress · 4m ago", or the bare
    /// "Created · 2d ago": history, not somebody's word, so no body of its
    /// own. A created note's body is the title again, so it isn't repeated.
    private var quiet: some View {
        BoardTick { now in
            let ago = TaskRow.ago(now.timeIntervalSince(note.at))
            let kind = note.kind == .created ? style.label : "\(style.label): \(note.displayBody)"
            Text(TaskProse.noteLine(kind: kind, byline: "", ago: ago))
                .help(note.at.formatted(date: .abbreviated, time: .standard))
        }
        .font(TaskTypography.meta)
        .foregroundStyle(.tertiary)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
