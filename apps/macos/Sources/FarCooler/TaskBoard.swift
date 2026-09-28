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
    /// neither is.
    static func landing(for pane: BoardPane, in fleet: [Worktree]) -> ContentView.Selection? {
        let host = pane.worktree.host ?? ""
        guard
            let worktree = fleet.first(where: {
                ($0.host ?? "") == host && $0.id == pane.worktree.id
            })
        else { return nil }
        if worktree.terminals.contains(where: { $0.id == pane.terminal.id }) {
            return .terminal(host: host, worktree: worktree.id, terminal: pane.terminal.id)
        }
        return .worktree(host: host, id: worktree.id)
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
/// **Two forms, chosen by the board's own width** (owner decision 3, spec
/// §5): a status-sectioned list when narrow and the kanban when wide,
/// measured with a `GeometryReader` on the board and never read from the
/// window, which the board shares with a sidebar and other columns. The
/// header's toggle forces either, kept per device and per board. Every
/// status is drawn in both: a collapsed "Backlog 0" in the list, and an
/// empty column in the kanban.
struct TaskBoardView: View {
    @ObservedObject var store: TaskBoardStore
    @ObservedObject var client: DaemonClient
    let agents: BoardAgents
    /// Go to a pane working a task. The window's, because only the window can
    /// change what is selected.
    let onGoTo: (BoardPane) -> Void
    /// Where the form and the collapsed sections are kept. The app's own
    /// defaults, except in a test.
    let defaults: UserDefaults

    /// The toggle's choice for this board: read from `defaults` in `init`,
    /// so a board forced to one form never draws a frame in the other before
    /// its choice is known, and again when the view is handed another board.
    @State private var choice: BoardForm.Choice
    /// The form last drawn, which is what the hysteresis keeps between 868
    /// and 892 pt.
    @State private var drawn: BoardForm?
    /// The list's collapsed sections, read the same way.
    @State private var collapsed: Set<TaskStatus>
    @State private var newTaskOpen = false

    init(
        store: TaskBoardStore, client: DaemonClient, agents: BoardAgents,
        onGoTo: @escaping (BoardPane) -> Void, defaults: UserDefaults = .standard
    ) {
        self.store = store
        self.client = client
        self.agents = agents
        self.onGoTo = onGoTo
        self.defaults = defaults
        _choice = State(
            initialValue: BoardForm.Choice.read(
                host: store.hostKey, workspace: store.workspace.id, from: defaults))
        _collapsed = State(
            initialValue: BoardForm.collapsed(
                host: store.hostKey, workspace: store.workspace.id, from: defaults))
    }

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
            } else {
                forms
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
        .onChange(of: remembered) { _, key in
            choice = BoardForm.Choice.read(host: key.host, workspace: key.workspace, from: defaults)
            collapsed = BoardForm.collapsed(host: key.host, workspace: key.workspace, from: defaults)
        }
        // A card is open only while its board is on screen. A board that goes
        // away under an open card — its project removed, its runner gone, or
        // a command that selected something else — would otherwise leave the
        // card set on a store the sidebar row still holds: re-read with a
        // `task show` on every event nobody sees, and back on screen the next
        // time the board is.
        .onDisappear { store.opened = nil }
        .sheet(item: $store.opened) { row in
            TaskCardSheet(
                row: row, detail: store.detail, question: store.question,
                agents: agents.live(for: row), canAnswer: store.offersWrites,
                onAnswer: { body in await store.answer(row, with: body) },
                draft: TaskCard.Draft(
                    read: { store.draft(for: $0) }, write: { store.setDraft($1, for: $0) }),
                onGoTo: { pane in
                    store.opened = nil
                    onGoTo(pane)
                },
                onClose: { store.opened = nil })
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
        HStack(spacing: 10) {
            Text(store.title).font(WorkspaceStyle.sectionTitle)
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
            if store.offersWrites {
                Button {
                    newTaskOpen = true
                } label: {
                    Image(systemName: "plus")
                }
                .help("New Task…")
                .accessibilityLabel("New Task…")
                .accessibilityIdentifier("board-new-task")
                .popover(isPresented: $newTaskOpen, arrowEdge: .bottom) {
                    NewTaskForm(
                        onCreate: { title in await store.createTask(title: title) },
                        onClose: { newTaskOpen = false })
                }
            }
            formToggle
            Button("Refresh") { Task { await store.reload() } }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(WorkspaceStyle.paneChrome)
    }

    /// `≡` List and `▦` Kanban. Clicking one forces it; clicking the one
    /// forced goes back to Automatic, which is also in the control's menu.
    /// The form on screen is always marked, and a forced one more strongly.
    private var formToggle: some View {
        HStack(spacing: 1) {
            formButton(.list, symbol: "list.bullet", name: "List")
            formButton(.kanban, symbol: "rectangle.split.3x1", name: "Kanban")
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(0.06)))
        .contextMenu {
            Picker("Board Layout", selection: chosen) {
                Text("Automatic").tag(BoardForm.Choice.auto)
                Text("List").tag(BoardForm.Choice.list)
                Text("Kanban").tag(BoardForm.Choice.kanban)
            }
            .pickerStyle(.inline)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-form-toggle")
    }

    private func formButton(_ form: BoardForm, symbol: String, name: String) -> some View {
        let forced = choice.forced == form
        let shown = drawn == form
        return Button {
            chosen.wrappedValue = choice.choosing(form)
        } label: {
            Image(systemName: symbol)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                .frame(width: 24, height: 18)
                .foregroundStyle(forced ? Color.white : shown ? Color.primary : Color.secondary)
                .background(
                    RoundedRectangle(cornerRadius: 4)
                        .fill(
                            forced
                                ? Color.accentColor
                                : shown ? Color.primary.opacity(0.1) : Color.clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(
            forced
                ? "Always \(name). Click again to choose by width."
                : "Show as \(name == "List" ? "a list" : "a kanban")")
        .accessibilityLabel(name)
        .accessibilityValue(forced ? "Chosen" : shown ? "Shown" : "")
    }

    /// The toggle's choice, kept on this device as it changes.
    private var chosen: Binding<BoardForm.Choice> {
        Binding(
            get: { choice },
            set: { new in
                choice = new
                new.write(host: store.hostKey, workspace: store.workspace.id, in: defaults)
            })
    }

    // MARK: - The two forms

    /// The form for the board's own width, measured here.
    private var forms: some View {
        GeometryReader { geometry in
            let form = BoardForm.resolve(
                width: geometry.size.width, previous: drawn, forced: choice)
            Group {
                switch form {
                case .list: list
                case .kanban: kanban
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height, alignment: .topLeading)
            .onChange(of: form, initial: true) { _, new in drawn = new }
        }
    }

    /// Whether the board has been read and has nothing on it at all.
    private var isEmpty: Bool {
        store.hasRead && store.board.rows.isEmpty && store.board.unreadable.isEmpty
    }

    /// Above either form on an empty board: what it is, and the way to put a
    /// task on it. The form is still drawn under it, every status at 0.
    private var emptyNote: some View {
        HStack(spacing: 10) {
            Text("Nothing on this board yet.")
                .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
            Spacer(minLength: 0)
            if store.offersWrites {
                Button("New Task…") { newTaskOpen = true }
                    .accessibilityIdentifier("board-empty-new-task")
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(WorkspaceStyle.document))
    }

    private var list: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 6) {
                if isEmpty { emptyNote.padding(.bottom, 6) }
                ForEach(store.board.sections) { section in
                    TaskListSection(
                        section: section,
                        expanded: BoardForm.isExpanded(section, collapsed: collapsed),
                        onToggle: { toggle(section.status) },
                        store: store, agents: agents, onGoTo: onGoTo)
                }
                if !store.board.unreadable.isEmpty {
                    UnreadableColumnView(rows: store.board.unreadable, width: nil)
                }
            }
            .padding(BoardForm.boardPadding)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-list")
        // From the form itself, not from the value the switch reads, so what
        // is published is what was drawn.
        .preference(key: BoardFormPreference.self, value: .list)
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

    private var kanban: some View {
        VStack(alignment: .leading, spacing: 0) {
            if isEmpty {
                emptyNote
                    .padding([.horizontal, .top], BoardForm.boardPadding)
            }
            ScrollView(.horizontal) {
                HStack(alignment: .top, spacing: BoardForm.columnSpacing) {
                    // `sections`, not `columns`: every status, whatever the
                    // board was read with.
                    ForEach(store.board.sections) { column in
                        TaskColumnView(column: column, store: store, agents: agents, onGoTo: onGoTo)
                    }
                    if !store.board.unreadable.isEmpty {
                        UnreadableColumnView(rows: store.board.unreadable, width: BoardForm.columnWidth)
                    }
                }
                .padding(BoardForm.boardPadding)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-kanban")
        .preference(key: BoardFormPreference.self, value: .kanban)
    }
}

/// The form a board is drawn in, published by the form that was drawn, for
/// whatever holds it: the window, which can say which form is on screen, and
/// `BoardFormWiringTests`, which reads it from a board it can't see. Nil for a
/// board not drawn yet.
struct BoardFormPreference: PreferenceKey {
    static let defaultValue: BoardForm? = nil
    static func reduce(value: inout BoardForm?, nextValue: () -> BoardForm?) {
        value = nextValue() ?? value
    }
}

/// New Task…: a title, and the button that files it on this board.
///
/// Only the title, which is all `task create` needs and all a board card
/// shows; the intent and acceptance are the orchestrator's to write, or the
/// CLI's for anyone who wants them now.
private struct NewTaskForm: View {
    let onCreate: (String) async -> Bool
    let onClose: () -> Void

    @State private var title = ""
    @State private var sending = false
    @State private var failed = false

    /// The CLI's limit on a title (`task create --title`).
    private static let limit = 200

    private var trimmed: String { title.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("New Task").font(.headline)
            TextField("Title", text: $title)
                .textFieldStyle(.roundedBorder)
                .frame(width: 320)
                .onSubmit(send)
            if trimmed.count > Self.limit {
                Text("A title can be at most \(Self.limit) characters.")
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

    private var canSend: Bool { !sending && !trimmed.isEmpty && trimmed.count <= Self.limit }

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

    /// Needs Decision is the one status waiting on the person reading, and
    /// the only one drawn in the accent color, as in the kanban.
    private var leads: Bool { section.status == .needsDecision }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button(action: onToggle) {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                        .foregroundStyle(.secondary)
                        .opacity(BoardForm.canExpand(section) ? 1 : 0.35)
                    Text(section.title)
                        .font(WorkspaceStyle.sectionTitle)
                        .foregroundStyle(
                            section.count == 0
                                ? Color.secondary : leads ? Color.accentColor : Color.primary)
                    Text("\(section.count)")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                }
                .padding(.vertical, 3)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!BoardForm.canExpand(section))
            .animation(Motion.snap, value: expanded)
            .accessibilityLabel("\(section.title), \(section.count)")
            .accessibilityValue(BoardForm.canExpand(section) ? (expanded ? "Expanded" : "Collapsed") : "")
            .accessibilityIdentifier("board-section-\(section.id)")
            if expanded {
                VStack(spacing: 6) {
                    ForEach(section.rows) { row in
                        TaskListRow(
                            row: row, prominent: leads, store: store,
                            live: agents.live(for: row), presence: agents.presence(for: row),
                            onGoTo: onGoTo)
                    }
                }
            }
        }
    }
}

/// One card in the list form, after the iPhone's row: its key, title, what
/// it asks of you, how long it has sat, how much of its acceptance holds,
/// and the way to its agent, which sits beside the words rather than under
/// them, since a list row has the width.
private struct TaskListRow: View {
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
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            AgentPill(live: live, presence: presence, onGoTo: onGoTo)
        }
        .padding(9)
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
        .onTapGesture { Task { await store.open(row) } }
        .contextMenu { TaskRowMenu(row: row, live: live, store: store, onGoTo: onGoTo) }
        .accessibilityIdentifier("board-row-\(row.key)")
    }
}

/// A card's context menu, the same in both forms: the way to its agent, and
/// the moves the model offers.
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
                Text("\(column.count)")
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
        .frame(width: BoardForm.columnWidth)
        .padding(BoardForm.columnPadding)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(leads ? Color.accentColor.opacity(0.07) : WorkspaceStyle.document))
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

/// One card. Internal rather than private for `BoardCardTickTests`, which
/// draws it.
struct TaskCardRow: View {
    let row: TaskRow
    let prominent: Bool
    @ObservedObject var store: TaskBoardStore
    /// The panes working this task, and what the card says about them. Both
    /// decided by AgentKit's rule; see `BoardAgents`.
    let live: [BoardPane]
    let presence: TaskAgentPresence
    let onGoTo: (BoardPane) -> Void

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
                //
                // On the board's tick, like the sentence and the border: a
                // card crosses a day of silence on a quiet board, with no data
                // change to redraw it, and its three stale marks turn together.
                BoardTick { now in
                    if row.staleness(at: now) == .stale {
                        Image(systemName: "clock.badge.exclamationmark")
                            .foregroundStyle(.orange)
                            .font(.system(size: WorkspaceStyle.PaneText.body))
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
            // Both sentences come from the model. Composing either here would
            // put the board's only real copy where nothing reads it back.
            if let call = row.callToAction {
                Text(call)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                    .foregroundStyle(Color.accentColor)
            }
            CardTimeLines(
                row: row,
                staleSize: WorkspaceStyle.PaneText.secondary,
                timeSize: WorkspaceStyle.PaneText.minimum)
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
            BoardTick { now in
                let stale = row.staleness(at: now) == .stale
                RoundedRectangle(cornerRadius: 8)
                    .strokeBorder(
                        stale ? Color.orange.opacity(0.55) : WorkspaceStyle.hairline,
                        lineWidth: stale ? 1 : 0.5)
            }
        )
        .contentShape(Rectangle())
        .onTapGesture { Task { await store.open(row) } }
        .contextMenu { TaskRowMenu(row: row, live: live, store: store, onGoTo: onGoTo) }
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
    /// A kanban column's width, or nil for the list, where it spans the board.
    let width: CGFloat?

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
            if width != nil { Spacer() }
        }
        .frame(width: width, alignment: .leading)
        .frame(maxWidth: width == nil ? .infinity : nil, alignment: .leading)
        .padding(BoardForm.columnPadding)
        .background(RoundedRectangle(cornerRadius: 10).fill(WorkspaceStyle.document))
    }
}

/// One task, opened, as a sheet over the board: its heading, and the card.
///
/// The sheet is the card's host until the task column replaces it (2D.1);
/// what the card says is `TaskCard`, which the column will host instead.
private struct TaskCardSheet: View {
    let row: TaskRow
    let detail: TaskDetailModel
    let question: TaskQuestion?
    /// The panes working this task. Going to one closes the card first — the
    /// card is a sheet, and a sheet left up would sit over the pane you went
    /// to.
    let agents: [BoardPane]
    let canAnswer: Bool
    let onAnswer: (String) async -> Bool
    let draft: TaskCard.Draft
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
                TaskCard(
                    row: row, detail: detail, question: question, canAnswer: canAnswer,
                    onAnswer: onAnswer, draft: draft
                )
                .padding(14)
            }
        }
        .frame(minWidth: 560, minHeight: 480)
        .background(WorkspaceStyle.document)
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
/// Internal for `TaskCardTests`, which draws it. The sheet hosts it today, and
/// the task column will.
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
