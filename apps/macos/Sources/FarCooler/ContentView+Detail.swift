import AgentKit
import AppKit
import SwiftUI

/// The detail area: changes panes, board stores, shown layouts and the task tabs.
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    // MARK: - Detail

    /// `OrchestratorAdoption.offer` in `worktree`, as the fleet has it when
    /// the menu opens.
    func roleOffer(in worktree: Worktree) -> (Terminal) -> OrchestratorAdoption.Offer? {
        let store = store
        return { OrchestratorAdoption.offer(for: $0, in: worktree, host: worktree.host ?? "", fleet: store.fleet) }
    }

    /// This worktree's changes pane, if it has one open in the layout the
    /// detail draws for it, in a workspace or loose.
    ///
    /// Asked of that layout rather than of the worktree's terminals,
    /// because that is the question the toolbar button is answering: whether
    /// the arrangement you are looking at is showing the diff. A changes pane
    /// in a layout two tabs over is not on screen, and offering to close it
    /// from here would close something the window is not showing. Never the
    /// Orchestrator column's: nothing is split into the orchestrator's
    /// window (ov-78).
    func changesPane(in ws: Worktree) -> Terminal? {
        guard let group = worktreeColumn(of: ws)?.group else { return nil }
        let inGroup = Set(group.panes.map(\.id))
        return ws.terminals.first { inGroup.contains($0.id) && $0.isChangesPane }
    }

    /// The layout the detail draws for `ws` opened whole: beside a board in a
    /// workspace, or on its own. Nil when it draws none, which is the
    /// case for a worktree with no terminal.
    private func worktreeColumn(of ws: Worktree) -> ShownLayout? {
        shown.first { $0.column == .worktree && $0.host == (ws.host ?? "") && $0.worktree.id == ws.id }
    }

    /// Open this worktree's diff, or close the one the toolbar says is open.
    func toggleChangesPane(in ws: Worktree) {
        if let open = changesPane(in: ws) {
            // Killing the pane is the whole of it. The record goes with it, but
            // the daemon does that — closing a diff from tmux's own `⌃B x` has
            // to leave as little behind as closing it from here, so the reaping
            // lives on the host where both can reach it, not in this button.
            Task {
                await act(.close, on: ws, target: open.id, subject: Self.quoted(open)) { c in
                    await c.stop(terminal: open.short)
                }
            }
            return
        }
        showChanges(in: ws)
    }

    /// Show Changes: the toolbar's, a worktree row's menus' and its card's
    /// (ov-78). Reachable whether or not the worktree has a terminal open.
    ///
    /// With the worktree's layout open, a split of its
    /// focused pane, exactly as `⌃B %` and a drop on an edge are: the daemon
    /// has one verb for "a new pane, here, running this", and a changes pane
    /// is that verb with a different preset. With no layout there (no
    /// terminal, or the main checkout named from the Orchestrator column),
    /// its changes pane, one it has or a new one in a window of its own,
    /// opened beside the board. Never a split of the orchestrator's
    /// window.
    private func showChanges(in ws: Worktree) {
        if let open = changesPane(in: ws) {
            focus(PaneRef(host: ws.host ?? "", worktree: ws.id, terminal: open.id))
            return
        }
        if let layout = worktreeColumn(of: ws) {
            Task {
                let groups = await act(.openChanges, on: ws, default: []) { c in
                    // `beside: nil` means the focused pane of the layout
                    // named, which is the daemon's own default and the same
                    // anchor `⌃B %` uses.
                    await c.split(ws, beside: nil, side: .right, preset: "changes", layout: layout.group.id)
                }
                reveal(groups, in: ws)
            }
            return
        }
        // Where it opens: in the workspace on screen when the toolbar named
        // this worktree, as its row does otherwise.
        let stays = WorkspaceScreen.changesTarget(
            selection, in: store.fleet, repositories: repositoryIDs(ws.host ?? ""))?.id == ws.id
        let listed = worktree(host: ws.host ?? "", id: ws.id) ?? ws
        Task {
            let host = listed.host ?? ""
            let layouts = store.client(for: listed)?.layouts[listed.id]
            // One it has already, unless it's in an orchestrator's window.
            let existing = listed.terminals.first {
                $0.isChangesPane
                    && WorkspaceScreen.seat(sharedBy: $0.id, in: listed, fleet: store.fleet, layouts: layouts) == nil
            }
            var pane = existing
            if pane == nil {
                pane = await act(
                    .openChanges, on: listed, default: nil as Terminal?,
                    { c in await c.createTerminal(in: listed, preset: "changes", title: "Changes") })
            }
            guard let pane else { return }
            if stays, case .workspace(host, let id, _)? = selection {
                selection = .workspace(host: host, workspace: id, focus: .worktree(listed.id, terminal: pane.id))
            } else {
                selection = Self.opening(listed, terminal: pane.id, in: store.fleet)
            }
            keyPane = PaneRef(host: host, worktree: listed.id, terminal: pane.id)
        }
    }

    /// Show Changes on `ws`'s row, or nil where it can't be: a runner that
    /// can't be acted on, or that has said it can't read changes.
    func showChangesAction(for ws: Worktree, usable: Bool) -> (() -> Void)? {
        guard usable, store.client(for: ws)?.changesSupported != false else { return nil }
        return { showChanges(in: ws) }
    }

    /// The repositories `host` lists, by id: what an implicit workspace is.
    func repositoryIDs(_ host: String) -> [String] {
        store.clients[host]?.repositories.map(\.id) ?? []
    }

    /// One board store per workspace.
    ///
    /// Cached on the client for the reason `changesStore(for:client:)` gives:
    /// `FleetStore` drops a `DaemonClient` when its runner leaves and builds a
    /// fresh one when it comes back, and a store held over from the old one
    /// would go on talking to a connection nobody is answering. A workspace
    /// renamed since is a new store too, so the board's title follows it.
    func boardStore(for workspace: WorkspaceSummary, client: DaemonClient, host: String)
        -> TaskBoardStore
    {
        let key = "\(host)/\(workspace.id)"
        if let existing = boardStores[key], Self.keeps(existing, for: workspace, client: client) {
            if existing.onChoose == nil {
                existing.onChoose = { row in chooseTask(row.id, host: host, workspace: workspace.id, glance: false) }
                existing.onGlance = { row in chooseTask(row.id, host: host, workspace: workspace.id, glance: true) }
            }
            return existing
        }
        let made = TaskBoardStore(client: client, workspace: workspace)
        made.onChoose = { row in chooseTask(row.id, host: host, workspace: workspace.id, glance: false) }
        made.onGlance = { row in chooseTask(row.id, host: host, workspace: workspace.id, glance: true) }
        // Outside the view update, because creating it IS a state change and
        // SwiftUI is reading that state right now. See `changesStore`.
        DispatchQueue.main.async { boardStores[key] = made }
        return made
    }

    /// Whether `existing` still serves `workspace`'s board on `client`.
    ///
    /// Compared by what the board shows — its name — and not the whole
    /// summary: an orchestrator starting or stopping changes the summary, and
    /// a new store would close the card that is open and read the board again.
    static func keeps(
        _ existing: TaskBoardStore, for workspace: WorkspaceSummary, client: DaemonClient
    ) -> Bool {
        existing.client === client && existing.workspace.id == workspace.id
            && existing.workspace.name == workspace.name
            && existing.workspace.isImplicit == workspace.isImplicit
    }

    /// The board stores still worth holding, given the runners there are now.
    /// See `TaskBoardStore.isHeld(by:)`.
    ///
    /// Before this, `boardStores` only ever grew: every repository a runner
    /// had ever listed, and every client a runner had ever had, stayed held
    /// for the life of the window. A dropped store's open card is closed on
    /// the way out, so a view still holding it for a moment has nothing to
    /// put back on screen.
    static func heldBoardStores(
        _ stores: [String: TaskBoardStore], clients: [String: DaemonClient]
    ) -> [String: TaskBoardStore] {
        let held = stores.filter { $0.value.isHeld(by: clients) }
        for (key, dropped) in stores where held[key] == nil { dropped.opened = nil }
        return held
    }

    func pruneBoardStores() {
        let held = Self.heldBoardStores(boardStores, clients: store.clients)
        if held.count != boardStores.count { boardStores = held }
    }

    /// Whose board ⇧⌘B selects.
    ///
    /// The workspace of whatever the window is showing — Main's for an
    /// unclaimed worktree — and the only repository's Main when nothing is
    /// selected. Never a guess between several: opening the wrong board looks
    /// exactly like a workspace with somebody else's work on it. See
    /// `ContentView.boardWorkspace(for:in:)`.
    var boardTarget: (host: String, workspace: WorkspaceSummary)? {
        if let selection, let host = selection.host {
            if let found = Self.boardWorkspace(for: selection, in: store.fleet) {
                return (host, found)
            }
            // A CLI too old to send `repository_id`, on a runner without
            // workspaces: the repository is found by the name it does send.
            if let ws = currentWorktree, let client = store.client(for: ws),
                let name = ws.repository,
                let match = client.repositories.first(where: { $0.displayName == name })
            {
                return (host, .implicit(repository: match.id))
            }
            return nil
        }
        let all = store.repositories
        guard all.count == 1, let only = all.first else { return nil }
        let main = store.fleet.runnerWorkspaces[only.host]?.first {
            $0.isMain && $0.repository == only.repository.id
        }
        return (only.host, main ?? .implicit(repository: only.repository.id))
    }

    /// The board a `.board` selection names, as its runner lists it now: a
    /// workspace it lists, or on a runner without workspaces the repository
    /// whose id it is. Nil once it is gone.
    func board(host: String, id: String) -> WorkspaceSummary? {
        guard let client = store.clients[host] else { return nil }
        if let listed = client.fleet.workspaces {
            return listed.first { $0.id == id }
        }
        guard client.repositories.contains(where: { $0.id == id }) else { return nil }
        return .implicit(repository: id)
    }

    /// One changes store per worktree.
    ///
    /// Cached on the client too, not just the worktree. `FleetStore` drops a
    /// `DaemonClient` when its runner leaves and builds a fresh one when it
    /// comes back, and a store held over from the old one would go on talking to
    /// a connection nobody is answering.
    func changesStore(for ws: Worktree, client: DaemonClient) -> ChangesStore {
        if let existing = changesStores[ws.id], existing.client === client { return existing }
        let made = ChangesStore(client: client, worktree: ws)
        // Assigned outside the view update, because creating it IS a state
        // change and SwiftUI is reading that state right now. An earlier version
        // wrote it from a Task, which rebuilt the store on every render and threw
        // away each load before it could finish — the panel sat permanently empty.
        DispatchQueue.main.async { changesStores[ws.id] = made }
        return made
    }

    /// What the title bar's find (`/`, ⌘P) searches for files (ov-189): the
    /// open task's worktree, else the one on screen, else the inspector's.
    var paletteWorktree: Worktree? {
        let lane = openTaskLane?.worktree.flatMap { id in selection?.host.flatMap { worktree(host: $0, id: id) } }
        return lane ?? currentWorktree ?? files.inspector
    }

    /// The Files clicked into, while on screen: what ⌘F and ⇧⌘L act on.
    var focusedFiles: FilesModel? {
        files.focused(shown: taskTab(for: selection) == .files ? openTaskLane?.worktree : files.inspectorID)
    }

    /// The task open, and the worktree its Files tab reads, if it has one.
    private var openTaskLane: (task: String, worktree: String?)? {
        guard case .workspace(let host, let workspace, .task(let id)?)? = selection else { return nil }
        let chosen = WorkspaceScreen.agent(of: id, host: host, in: store.fleet, chosen: chosenAgents[id])
        let row = boardStores["\(host)/\(workspace)"]?.board.rows.first { $0.id == id }
        return (id, row.flatMap { TaskColumnModel.worktree(of: $0, agent: chosen) } ?? chosen?.worktree.id)
    }

    /// Show `path` in `ws`'s Files (ov-189): on the open task's Files tab
    /// when the task works in `ws`, else in the inspector.
    func showInFiles(_ path: String, line: Int?, in ws: Worktree) {
        guard let client = store.client(for: ws) else { return }
        let lane = openTaskLane.flatMap { $0.worktree == ws.id ? $0.task : nil }
        if let lane { choose(.files, for: lane) }
        files.show(path, line: line, in: ws, client: client, onTaskTab: lane != nil)
    }

    /// Every layout the detail draws for `selection`: `WorkspaceScreen`'s,
    /// and of a workspace's, only the columns its width draws.
    func shownLayouts(for selection: Selection?) -> [ShownLayout] {
        let all = drawableLayouts(for: selection)
        guard let selection, let scene = workspaceScene(selection) else { return all }
        // The arrangement `WorkspaceView` draws, by the same rule: beside
        // the canvas the chat is on screen, seen and keyed whatever is
        // opened (train 1004r, P1).
        let arrangement = detailWidth.map { width in
            WorkspaceColumns.arrangement(
                width: width, canvas: splits(scene) ? canvasSizing : nil, opened: scene.opened != nil,
                hasConversation: scene.hasConversation, hasBoard: scene.board != nil && !navigatorHidden,
                boardExists: scene.board != nil, floating: navigatorFloating, focused: focusColumn)
        }
        return WorkspaceScreen.visible(all, arrangement: arrangement, taskTab: taskTab(for: selection))
    }

    /// Whether an agent is working `task`: what opens it on its Agent tab.
    private func agentWorking(_ task: String, host: String) -> Bool {
        WorkspaceScreen.agent(of: task, host: host, in: store.fleet, chosen: chosenAgents[task]) != nil
    }

    /// The tab the task `selection` names shows (ov-98), or Agent for
    /// anything else, which has no tabs.
    func taskTab(for selection: Selection?) -> TaskTab {
        guard case .workspace(let host, _, .task(let id)?)? = selection else { return .agent }
        return taskTabs.tab(for: id, agentWorking: agentWorking(id, host: host))
    }

    /// Whether `selection` has a task open, with its tabs.
    nonisolated static func taskOpen(_ selection: Selection?) -> Bool {
        if case .workspace(_, _, .task?)? = selection { return true }
        return false
    }

    /// Show `tab` for `task`, chosen; to the Agent tab, the keyboard goes
    /// with it, into the terminal. Leaving Changes, its diff gives up the
    /// Diff menu's keys.
    func choose(_ tab: TaskTab, for task: String) {
        taskTabs.choose(tab, for: task)
        if tab != .changes, changesFocus == task { changesFocus = nil }
        if tab == .agent { DispatchQueue.main.async { keyOpened() } }
    }

    /// ⌃⌘] and ⌃⌘[: the task open steps to its next or previous tab.
    func stepTaskTab(by offset: Int) {
        guard case .workspace(let host, _, .task(let id)?)? = selection else { return }
        var stepped = taskTabs
        stepped.step(id, by: offset, agentWorking: agentWorking(id, host: host))
        choose(stepped.tab(for: id, agentWorking: agentWorking(id, host: host)), for: id)
    }

    /// What the detail draws now.
    var shown: [ShownLayout] { shownLayouts(for: selection) }

    /// Every layout `selection` could draw, on screen or not: the
    /// conversation's included while it's kept hidden behind a task, so it's
    /// one terminal view whether it's selected or not.
    func drawableLayouts(for selection: Selection?) -> [ShownLayout] {
        func shown(_ selection: Selection?) -> [ShownLayout] {
            WorkspaceScreen.shown(
                selection, in: store.fleet,
                layouts: { host, worktree in store.clients[host]?.layouts[worktree] },
                repositories: { host in store.clients[host]?.repositories.map(\.id) ?? [] },
                chosen: { chosenAgents[$0] })
        }
        // A loose worktree beside a workspace's board keeps that
        // workspace's orchestrator mounted, ahead of the worktree.
        if case .looseWorktree(let host, _, _)? = selection, let scene = workspaceScene(selection!),
            scene.hasConversation, let board = scene.board
        {
            let conversation = shown(.workspace(host: host, workspace: board, focus: nil)).filter { $0.column == .conversation }
            return conversation + shown(selection)
        }
        return shown(selection)
    }

    /// `detail`, and a click on a task notice: its task, opened as the
    /// navigator opens one, once its runner is connected (ov-106). Its own
    /// property because `body`'s chain is already at the type checker's
    /// limit: one more modifier there and it gives up.
    var detailOpeningNotices: some View {
        detail
            .task(id: noticeOpener.pending?.id) { await openNoticedTask() }
            .modifier(windowRestore)
            // Files beside a worktree opened whole (ov-189). A task has its
            // own Files tab instead.
            .inspector(
                isPresented: Binding(
                    get: { files.inspectorOpen },
                    set: {
                        if !$0 {
                            files.inspector = nil
                            files.inspectorFolder = nil
                        }
                    })
            ) {
                FilesInspector(
                    routing: files, client: { store.client(for: $0) }, folderClient: { store.clients[$0] })
                    .inspectorColumnWidth(min: 420, ideal: 760, max: 1400)
            }
            .environment(\.openInFiles, OpenInFiles { ws, path, line in showInFiles(path, line: line, in: ws) })
            // Going somewhere else closes it: Files is beside the worktree
            // on screen, never a leftover from the last one.
            .onChange(of: selection) { _, now in
                files.follow(WorkspaceScreen.changesTarget(now, in: store.fleet, repositories: repositoryIDs(now?.host ?? "")))
            }
    }

    @ViewBuilder
    private var detail: some View {
        switch selection {
        case .needsYou:
            NeedsYouView(
                items: store.needsYou,
                olderRunners: store.hosts.filter { store.clients[$0]?.needsYouFromOlderRunner == true },
                unanswered: store.needsYouUnanswered,
                canAct: { item in
                    store.refusal(for: item.runner) == nil
                        && TaskBoardWrites.offered(by: store.clients[item.runner]?.daemonBuild)
                },
                onOpen: { open($0) },
                onAnswerAsk: { item, option in
                    guard let client = store.clients[item.runner], let terminal = item.terminal,
                        let ask = item.askID
                    else { return .failed }
                    return await client.answerAsk(terminal: terminal.id, request: ask, option: option)
                },
                onDecide: { item, body in
                    guard let client = store.clients[item.runner], let task = item.task,
                        let repository = item.repositoryID
                    else { return false }
                    return await client.answerDecision(key: task.key, body: body, repository: repository) == nil
                })

        // One branch for both, so a loose worktree and the workspace whose
        // board is beside it are one view: going from one to the other keeps
        // the board, its scroll and its visit, and moves on the spring.
        case .workspace, .looseWorktree:
            if let scene = selection.flatMap(workspaceScene) {
                workspaceDetail(scene)
                    .id(scene.key)
            } else {
                placeholder
            }

        case nil:
            placeholder
        }
    }

    /// What a workspace's detail draws for a selection: whose board, whether
    /// it has a conversation, and what's open beside the board.
    struct WorkspaceScene {
        var host: String
        /// The board's workspace: the selection's own, or for a loose
        /// worktree the one `boardWorkspace(for:in:)` finds, if any.
        var board: String?
        var summary: WorkspaceSummary?
        var hasConversation: Bool
        /// What's open beside the board, as a place: the pane a worktree
        /// names left out, so choosing another pane in it is the same one.
        var opened: Selection?
        /// The view's identity: another workspace is drawn afresh, not moved
        /// to.
        var key: String { "\(host)|\(board ?? "")" }
    }

    func workspaceScene(_ selection: Selection) -> WorkspaceScene? {
        Self.workspaceScene(
            selection, in: store.fleet, repositories: store.clients[selection.host ?? ""]?.repositories.map(\.id) ?? [])
    }

    /// The scene `selection` draws, from `fleet`: a workspace's own board
    /// and orchestrator; for a loose worktree, its repository's board
    /// (`boardWorkspace(for:in:)`) and that board's orchestrator, kept
    /// mounted, so neither the navigator nor the orchestrator comes and goes
    /// between the two.
    nonisolated static func workspaceScene(
        _ selection: Selection, in fleet: Fleet, repositories: [String]
    ) -> WorkspaceScene? {
        switch selection {
        case .workspace(let host, let id, let focus):
            let summary = WorkspaceScreen.workspace(id, host: host, in: fleet, repositories: repositories)
            return WorkspaceScene(
                host: host, board: id, summary: summary, hasConversation: summary.map { !$0.isImplicit } ?? false,
                opened: focus == nil ? nil : WorkspaceSelection.place(selection))
        case .looseWorktree(let host, _, _):
            let found = boardWorkspace(for: selection, in: fleet)
            return WorkspaceScene(
                host: host, board: found?.id, summary: found, hasConversation: found.map { !$0.isImplicit } ?? false,
                opened: WorkspaceSelection.place(selection))
        case .needsYou:
            return nil
        }
    }
}
