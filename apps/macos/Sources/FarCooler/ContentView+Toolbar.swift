import AgentKit
import AppKit
import SwiftUI

/// The title bar, the toolbar and the worktree menus.
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    // MARK: - The title bar, the toolbar and the worktree menus (ov-86)

    /// Whether this is the window menu commands are for: the key one, or
    /// the only one before its window is known.
    var isKeyWindow: Bool { windowBox.window?.isKeyWindow ?? true }

    /// The title bar's workspace switcher, naming where the window is.
    var workspaceSwitcher: WorkspaceSwitcherButton {
        let scene = selection.flatMap(workspaceScene)
        let repository = scene?.summary.flatMap { w in
            store.clients[scene?.host ?? ""]?.repositories.first { $0.id == (w.repository ?? w.id) }?.displayName
        } ?? ""
        let title: String = {
            if selection == .needsYou { return "Needs You" }
            guard let summary = scene?.summary else { return "Workspaces" }
            return summary.isImplicit ? "Main" : summary.name
        }()
        let entries = WorkspaceSwitcherMenu.entries(
            groups: WorkspaceNumbers.groups(in: store.fleet),
            current: scene.flatMap { s in s.board.map { (s.host, $0) } },
            waiting: { place in WorkspaceCounts.count(for: place.workspace, host: place.host, in: store.needsYou) },
            showsHosts: showHosts, needsYou: store.needsYou.count, offersNewWorkspace: !workspaceRepositories.isEmpty,
            status: store.reading.sentence, statusTrouble: store.reading.isTrouble, troubled: store.unhealthyHosts,
            folders: ReadOnlyFolders.groups(store.hosts.map { ($0, store.clients[$0]?.readOnlyFolders ?? []) }))
        return WorkspaceSwitcherButton(
            title: title, repository: scene?.summary == nil ? "" : repository, entries: entries,
            openRequest: switcherRequest, perform: { perform($0) })
    }

    /// Show Changes in the toolbar, where offered (`WorktreeToolbar`). Not
    /// on a runner that has already said it can't read changes at all: its
    /// daemon predates the feature, and the split would open a dead pane
    /// where a diff was asked for. For a worktree opened whole, or the main
    /// checkout beside the orchestrator (ov-78); not for a task, whose
    /// column shows its changes already (spec R3).
    var changesToolbarState: WorktreeToolbar.Changes? {
        guard
            let ws = WorkspaceScreen.changesTarget(
                selection, in: store.fleet, repositories: repositoryIDs(selection?.host ?? "")),
            store.client(for: ws)?.changesSupported != false
        else { return nil }
        return WorktreeToolbar.Changes(worktree: ws, open: changesPane(in: ws) != nil)
    }

    /// The worktree the toolbar's Show Changes and Show Files act on.
    private var toolbarWorktree: Worktree? {
        WorkspaceScreen.changesTarget(selection, in: store.fleet, repositories: repositoryIDs(selection?.host ?? ""))
    }

    /// The title bar's status area for the workspace on screen (ov-214):
    /// its orchestrator as the navigator's row says it, and its board. Nil
    /// with no workspace on screen.
    var titleStatusSource: TitleStatusSource? {
        guard let scene = selection.flatMap(workspaceScene), let summary = scene.summary,
            let client = store.clients[scene.host]
        else { return nil }
        let host = scene.host
        let orchestrator = scene.hasConversation ? navigatorOrchestrator(host: host, workspace: summary) : nil
        let read = scene.board == nil ? nil : boardStore(for: summary, client: client, host: host)
        let tasks = Set(read?.board.rows.map(\.id) ?? [])
        // The workspace's panes: its own, and any working one of its tasks.
        let panes = store.fleet.worktrees.filter { ($0.host ?? "") == host }.flatMap { worktree in
            worktree.terminals
                .filter { $0.workspace == summary.id || $0.taskId.map(tasks.contains) == true }
                .map { BoardPane(terminal: $0, worktree: worktree) }
        }
        return TitleStatusSource(
            orchestrator: orchestrator?.state, status: orchestrator?.status, nowDoing: orchestrator?.nowDoing,
            board: read,
            // The sidebar's Needs You count, by the one function both say it
            // with (ov-321 review H3).
            waiting: { column in needsYouCount(host: host, workspace: summary, client: client, columnCount: column) },
            seat: WorkspaceScreen.orchestrator(of: summary, host: host, in: store.fleet)?.terminal, panes: panes)
    }

    /// The status area with no workspace on screen: no orchestrator and no
    /// board, so just the field, for Go to Anything.
    static var noWorkspaceSource: TitleStatusSource { TitleStatusSource(orchestrator: nil, status: nil, nowDoing: nil) }

    /// What the title bar's field does (slice 4, ov-264): finds with the
    /// palette's index and runs what's chosen as the palette did.
    var titleConsoleActions: TitleConsoleActions {
        let worktrees = store.fleet.worktrees
        // Offered in a workspace, saying which way it goes (ov-321).
        let boardView: Bool? = selection.flatMap(workspaceScene) == nil ? nil : showsBoardList
        return TitleConsoleActions(
            find: { [paletteWorkspaces, paletteTasks, palettePlans, selectedPane, boardView] query in
                query.isEmpty
                    ? PaletteIndex.recent(in: worktrees)
                    : PaletteIndex.matching(
                        query, in: worktrees, current: selectedPane?.worktree,
                        currentTerminal: selectedPane.flatMap { pane in
                            worktrees.lazy.flatMap(\.terminals).first { $0.id == pane.terminal }
                        },
                        workspaces: paletteWorkspaces, tasks: paletteTasks, plans: palettePlans,
                        offersNewWorkspace: !workspaceRepositories.isEmpty,
                        boardView: boardView)
            },
            run: { perform($0) },
            files: FilesRouting.palette(paletteWorktree, client: paletteWorktree.flatMap { store.client(for: $0) }),
            current: selectedPane?.terminal)
    }

    /// Show Files in the toolbar (ov-189): drawn by `WorktreeToolbar` and
    /// counted by `titleStatusRoom`, from the one place.
    var filesToolbarState: WorktreeToolbar.Files? {
        files.toolbar(toolbarWorktree, client: toolbarWorktree.flatMap { store.client(for: $0) })
    }

    /// What the status area sizes itself around (`TitleStatusRoom`).
    var titleStatusRoom: TitleStatusRoom {
        let switcher = workspaceSwitcher
        return TitleStatusRoom(
            switcherTitle: switcher.title, switcherRepository: switcher.repository,
            editor: detailWorktree != nil, changes: changesToolbarState != nil, files: filesToolbarState != nil,
            trouble: RunnerStatusItem.label(troubles: runnerTroubles, stale: store.staleHosts, ahead: store.aheadHosts),
            needsYou: store.needsYou.count, backForward: selection != nil)
    }

    /// What the status area's parts do, by the routes the window already
    /// has: the orchestrator's row, ⌃⌘N within this workspace, a task's row.
    func titleStatusActions(_ consoleActions: TitleConsoleActions) -> TitleStatusActions {
        guard let scene = selection.flatMap(workspaceScene), let summary = scene.summary, let board = scene.board
        else { return TitleStatusActions(console: console, consoleActions: consoleActions) }
        let host = scene.host
        return TitleStatusActions(
            goToOrchestrator: { selectOrchestrator(keyboard: .conversation) },
            orchestratorMenu: scene.hasConversation ? orchestratorMenu(host: host, workspace: summary) : nil,
            nextNeedingYou: {
                let here = store.needsYou.filter { WorkspaceCounts.count(for: summary, host: host, in: [$0]) > 0 }
                if let next = NeedsYouNavigation.step(
                    lastOpened: lastAttention, items: here, fleet: store.fleet, showing: selection)
                {
                    open(next.item)
                }
            },
            openTask: { row in chooseTask(row.id, host: host, workspace: board, glance: false) },
            openLine: { line in openActivityLine(line, host: host, workspace: summary) },
            readSpend: {
                guard let client = store.clients[host] else { return .couldntRead }
                guard client.recordsAgentUsage else { return .needsUpdate }
                let read = await client.spendToday(
                    repository: summary.repository ?? summary.id, workspace: summary.boardWorkspace)
                return ActivitySpend.read(data: read.data, message: read.message)
            },
            console: console, consoleActions: consoleActions)
    }

    /// A row of the activity panel or the failed menu, opened: its pane, or
    /// its task on this workspace's board.
    private func openActivityLine(_ line: ActivityLine, host: String, workspace: WorkspaceSummary) {
        switch line.kind {
        case .agent(let terminal, let worktreeID):
            if let ws = worktree(host: host, id: worktreeID), let term = ws.terminals.first(where: { $0.id == terminal }) {
                go(to: BoardPane(terminal: term, worktree: ws))
            }
        case .subagent(let key), .queued(let key):
            guard let client = store.clients[host] else { return }
            let board = boardStore(for: workspace, client: client, host: host).board
            if let row = board.rows.first(where: { $0.key == key }) {
                chooseTask(row.id, host: host, workspace: workspace.id, glance: false)
            }
        }
    }

    /// The orchestrator's menu, for the status area: what its column's
    /// header offered before the header went (ov-214).
    private func orchestratorMenu(host: String, workspace: WorkspaceSummary) -> OrchestratorMenu {
        let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet)
        let canAct = store.refusal(for: host) == nil
        let column = ConversationColumn.state(
            seat: seat, isStarting: startingOrchestrators.isStarting(workspace, host: host),
            startedAt: orchestratorStartedAt["\(host)|\(workspace.id)"], now: Date())
        return OrchestratorMenu(
            seat: seat, charter: CharterAccess.of(workspace, host: host), canAct: canAct,
            onReplace: { harness in
                orchestratorReplacement = OrchestratorReplacement(host: host, workspace: workspace, harness: harness)
            },
            onShowCharter: { url in
                if !NSWorkspace.shared.open(url) {
                    errorBanner = "Couldn’t open \(workspace.name)’s charter. It may have been moved or deleted."
                }
            },
            onTogglePaneMode: {
                if let seat { Task { await togglePaneMode(seat.terminal, in: seat.worktree) } }
            },
            target: seat.flatMap { store.client(for: $0.worktree)?.target } ?? "",
            opensAsChat: preferences.preferChatMode,
            onRestart: {
                if let seat { Task { await run(.restart, on: seat.terminal, in: seat.worktree) } }
            },
            onStepDown: {
                if let seat { Task { await stepDown(seat) } }
            },
            wakeOnAnswer: workspace.wakeOnAnswer,
            onSetWakeOnAnswer: { on in
                guard let client = store.clients[host] else { return }
                Task {
                    if let refused = await client.setWakeOnAnswer(workspace, on: on) { errorBanner = refused }
                }
            },
            starts: ConversationColumn.offers(column, canAct: canAct).compactMap { offer in
                if case .start(let harness) = offer { return harness }
                return nil
            },
            onStart: { harness in startOrchestrator(workspace, host: host, harness: harness, replace: false) })
    }

    /// Back and Forward in the title bar (slice 3), gated as the menu items
    /// are (`focus.goesBack`, `focus.goesForward`): each steps through the
    /// jump bar's history (ov-192) as ⌃⌘← and ⌃⌘→ do, and Back with no
    /// history goes up a level.
    var backForward: BackForwardControl {
        let scene = selection.flatMap(workspaceScene)
        return BackForwardControl(
            canGoBack: jumpBar.history.canGoBack
                || Self.goesBack(focus: focusColumn, from: selection, trail: trail, board: scene?.board),
            canGoForward: jumpBar.history.canGoForward,
            rows: historyRows, back: { step(back: true) }, forward: { step(back: false) }, go: go(to:))
    }

    /// What a switcher item does, by the routes the rest of the window uses.
    func perform(_ command: SwitcherCommand) {
        switch command {
        case .go(let target): selection = target
        case .openFolder(let host, let name): files.openFolder(host: host, name: name)
        case .needsYou: selection = .needsYou
        case .newWorkspace: newWorkspaceName = NewWorkspaceName(name: "")
        case .newWorktree: run(.newWorktree)
        case .addRepository: showAddRepository = true
        case .addRunner: showAdd = true
        case .find: console.console.open(recents: true)
        case .runners:
            preferences.settingsTab = "machines"
            openSettings()
        case .reconnect(let host): store.reconnect(host)
        case .newCheckoutTerminal(let host, let id, let name):
            startMainTerminal(host: host, repositoryID: id, project: name)
        case .removeRepository(let host, let id, let name):
            guard let repo = repository(host: host, id: id, project: name) else { return }
            removeRepository = RepositoryToRemove(host: host, repository: repo)
        }
    }

    /// A worktree's menu (`WorktreeMenu.items`), on navigator rows, task rows
    /// and the breadcrumb.
    func worktreeMenu(for ws: Worktree) -> [WorktreeMenu.Item] {
        let host = ws.host ?? ""
        let listed = worktree(host: host, id: ws.id) ?? ws
        let usable = store.refusal(for: host) == nil
        let offer = roleOffer(in: listed)
        return WorktreeMenu.items(
            for: listed, usable: usable, showsChanges: showChangesAction(for: listed, usable: usable) != nil,
            moveTargets: Self.moveTargets(for: listed, in: store.fleet, assigns: Self.assigns(store)(listed)),
            adoptable: listed.terminals.filter { offer($0) == .use })
    }

    func perform(_ item: WorktreeMenu.Item, on ws: Worktree) {
        let host = ws.host ?? ""
        let listed = worktree(host: host, id: ws.id) ?? ws
        switch item {
        case .open:
            trail = nil
            open(listed, terminal: nil)
        case .openInEditor: openInPreferredEditor(listed)
        case .showChanges: showChangesAction(for: listed, usable: store.refusal(for: host) == nil)?()
        case .newTerminal: newTerminal(in: listed)
        case .move(let id, _):
            if let target = store.fleet.runnerWorkspaces[host]?.first(where: { $0.id == id }) { move(listed, to: target) }
        case .useAsOrchestrator(let terminal, _):
            if let term = listed.terminals.first(where: { $0.id == terminal }) {
                Task { await run(.useAsOrchestrator, on: term, in: listed) }
            }
        case .hide: Task { await act(.hide, on: listed) { c in await c.hideWorktree(listed.short) } }
        case .unhide: Task { await act(.unhide, on: listed) { c in await c.unhideWorktree(listed.short) } }
        case .remove, .dismiss: removeWorktree = listed
        }
    }

    /// The runners `FleetStore.unhealthyHosts` names, with what's wrong
    /// with each, for the toolbar's runner item.
    var runnerTroubles: [RunnerStatusItem.Trouble] {
        store.unhealthyHosts.compactMap { host in
            RunnerStatusItem.problem(store.state(of: host)).map { RunnerStatusItem.Trouble(host: host, problem: $0) }
        }
    }

    /// What a line of the runner item's menu does. The update opens its
    /// card from the item itself (`RunnerStatusMenu`).
    func perform(_ entry: RunnerStatusItem.Entry) {
        switch entry {
        case .reconnect(let host): store.reconnect(host)
        case .reconnectAll: for trouble in runnerTroubles { store.reconnect(trouble.host) }
        case .runners:
            preferences.settingsTab = "machines"
            openSettings()
        case .note, .update, .separator: break
        }
    }

    /// The workspace's worktrees in the board list's order, for the scene
    /// the selection draws: what ⌃⌘↑ and ⌃⌘↓ walk and the breadcrumb's
    /// menu lists.
    func worktreeEntries(_ scene: WorkspaceScene) -> [WorkspaceWorktrees.Entry] {
        guard let summary = scene.summary, let client = store.clients[scene.host] else { return [] }
        let board = boardStore(for: summary, client: client, host: scene.host).board
        return WorkspaceWorktrees.entries(in: summary, host: scene.host, board: board, fleet: store.fleet)
    }

    /// ⌃⌘↓ and ⌃⌘↑: the next or previous worktree in the workspace on
    /// screen, in the board list's order.
    func stepWorktree(by offset: Int) {
        guard let scene = selection.flatMap(workspaceScene), let board = scene.board else { return }
        if let next = WorkspaceWorktrees.step(
            from: selection, by: offset, in: worktreeEntries(scene), host: scene.host, workspace: board,
            fleet: store.fleet)
        {
            trail = nil
            navigate(to: next)
        }
    }

    /// The board list's worktrees: each task's name, and the loose ones
    /// under Worktrees.
    func boardWorktrees(
        host: String, workspace: WorkspaceSummary, client: DaemonClient, board: TaskBoardModel
    ) -> BoardWorktrees {
        let loose = WorkspaceWorktrees.loose(in: workspace, host: host, board: board, fleet: store.fleet)
        let usable = store.refusal(for: host) == nil
        let repository = client.repositories.first { $0.id == (workspace.repository ?? workspace.id) }
        let terminals = projectTerminals(host: host, workspace: workspace, usable: usable)
        return BoardWorktrees(
            byTask: WorkspaceWorktrees.taskWorktrees(on: board, host: host, in: store.fleet),
            shown: loose.shown, hidden: loose.hidden,
            // A project terminal open lights its own row, not the checkout's.
            selected: Self.openedWhole(selection).map(\.worktree)
                ?? (terminals.selected == nil ? WorkspaceScreen.namedTerminal(selection).map(\.worktree) : nil),
            onOpen: { worktree in glance(at: worktree) },
            // A worktree twice in the fleet keeps its first; never a trap (ov-267 L14).
            worktreeTerminals: Dictionary(
                loose.shown.map { ($0.id, BoardWorktrees.terminals(of: $0, in: store.fleet)) },
                uniquingKeysWith: { first, _ in first }),
            selectedTerminal: terminals.selected == nil ? WorkspaceScreen.namedTerminal(selection).map(\.terminal) : nil,
            onOpenTerminal: { worktree, terminal in glance(at: worktree, terminal: terminal.id) },
            onNew: usable ? repository.map { repo in { newWorktree(host: host, project: repo.displayName) } } : nil,
            onUnhide: usable ? { ws in Task { await act(.unhide, on: ws) { c in await c.unhideWorktree(ws.short) } } } : nil,
            menu: { worktreeMenu(for: $0) },
            perform: { item, ws in perform(item, on: ws) },
            terminals: terminals,
            unclaimed: BoardWorktrees.unclaimed(loose.shown, in: store.fleet))
    }

    /// The navigator's Terminals section for `workspace` (ov-178): its
    /// repository's own terminals, in the main checkout every one of its
    /// workspaces shares, each opened in this workspace, with the keyboard.
    private func projectTerminals(host: String, workspace: WorkspaceSummary, usable: Bool) -> ProjectTerminals {
        guard let checkout = ProjectTerminals.checkout(for: workspace, host: host, in: store.fleet) else { return .none }
        let terminals = ProjectTerminals.terminals(in: checkout, fleet: store.fleet)
        let named = WorkspaceScreen.namedTerminal(selection)
        return ProjectTerminals(
            checkout: checkout, terminals: terminals,
            selected: named.flatMap { open in
                open.host == host && open.worktree == checkout.id && terminals.contains { $0.id == open.terminal }
                    ? open.terminal : nil
            },
            onOpen: { terminal in
                trail = nil
                focusColumn = false
                keyboardOnBoard = false
                openProjectTerminal(terminal, in: checkout, host: host, workspace: workspace)
            },
            onAction: { action, terminal in Task { await run(action, on: terminal, in: checkout) } },
            onNew: usable ? { Task { await openShell(besideOrchestratorIn: checkout, workspace: workspace.id) } } : nil,
            canRename: store.clients[host]?.daemonBuild?.can(.terminalNames) == true,
            onThisMac: host.isEmpty)
    }

    /// Show one of the project's terminals beside `workspace`'s board.
    func openProjectTerminal(
        _ terminal: Terminal, in checkout: Worktree, host: String, workspace: WorkspaceSummary
    ) {
        navigate(
            to: .workspace(host: host, workspace: workspace.id, focus: .worktree(checkout.id, terminal: terminal.id)),
            key: PaneRef(host: host, worktree: checkout.id, terminal: terminal.id))
    }

    /// ↑ or ↓ in the navigator onto `item`: it's selected, and the
    /// navigator keeps the keyboard, to go on (ov-92).
    func step(to item: NavigatorItem, host: String, workspace: WorkspaceSummary) {
        switch item {
        case .orchestrator:
            selectOrchestrator(keyboard: .board)
        case .task(let id):
            chooseTask(id, host: host, workspace: workspace.id, glance: true)
        case .unread(let line):
            chooseTask(BoardSummaryStrip.task(ofLine: line), host: host, workspace: workspace.id, glance: true)
        case .worktree(let id):
            if let found = worktree(host: host, id: id) { glance(at: found) }
        case .terminal(let id):
            // ↑ or ↓ onto a project terminal: shown, the navigator keeping the keyboard.
            guard let checkout = ProjectTerminals.checkout(for: workspace, host: host, in: store.fleet),
                let terminal = ProjectTerminals.terminals(in: checkout, fleet: store.fleet).first(where: { $0.id == id })
            else { return }
            trail = nil
            let kept = WorkspaceNavigation.boardStep(.choose(glance: true), from: boardState)
            if kept.keyboard == .board { boardKeyboardPending = true }
            focusColumn = kept.focus
            openProjectTerminal(terminal, in: checkout, host: host, workspace: workspace)
            key(kept.keyboard)
        }
    }

    /// A loose worktree in the navigator chosen, by a click or ↑ or ↓, or a
    /// terminal under it (ov-267): it opens in the main area, and the
    /// navigator keeps the keyboard, as a task's row does.
    func glance(at worktree: Worktree, terminal: String? = nil) {
        trail = nil
        let step = WorkspaceNavigation.boardStep(.choose(glance: true), from: boardState)
        if step.keyboard == .board { boardKeyboardPending = true }
        focusColumn = step.focus
        open(worktree, terminal: terminal)
        key(step.keyboard)
    }

    /// The breadcrumb's worktree segment for `place`: standing for a
    /// worktree opened whole, or after a task's crumb, that task's own
    /// (`WorkspaceWorktrees.segment`).
    func worktreeCrumb(for place: Selection, scene: WorkspaceScene) -> WorktreeCrumb? {
        guard let board = scene.board else { return nil }
        // A project terminal is the breadcrumb's last crumb by its own name,
        // not a worktree with a menu of the workspace's (ov-234).
        if case .workspace(let host, _, .worktree(let id, let terminal?)?) = place,
            ProjectTerminals.name(of: terminal, in: worktree(host: host, id: id), fleet: store.fleet) != nil
        {
            return nil
        }
        let entries = worktreeEntries(scene)
        let current = WorkspaceSelection.samePlace(place, selection) ? selection : place
        let rows = scene.summary.flatMap { summary in
            store.clients[scene.host].map { boardStore(for: summary, client: $0, host: scene.host).board.rows }
        } ?? []
        guard let current,
            let segment = WorkspaceWorktrees.segment(
                for: current, trail: current == selection ? trail : nil, entries: entries,
                taskWorktrees: { id in
                    rows.first { $0.id == id }.map { WorkspaceWorktrees.worktrees(of: $0, host: scene.host, in: store.fleet) }
                        ?? []
                },
                name: { worktree(host: $0, id: $1)?.task }, host: scene.host, workspace: board, fleet: store.fleet)
        else { return nil }
        // The worktree it stands for, for its own menu's items.
        let named: Worktree? = {
            switch place {
            case .workspace(let host, _, .worktree(let id, _)?), .looseWorktree(let host, let id, _):
                return worktree(host: host, id: id)
            default:
                return nil
            }
        }()
        let help =
            segment.isWorkspace
            ? WorktreeCrumb.workspaceHelp
            : segment.opens != nil ? "Open \(segment.title)" : "Go to one of this task’s worktrees"
        return WorktreeCrumb(
            title: segment.title, isHere: segment.isHere, tasks: segment.tasks, loose: segment.loose,
            worktree: named?.task, actions: named.map { worktreeMenu(for: $0).filter { $0 != .open } } ?? [],
            perform: { item in if let named { perform(item, on: named) } }, opens: segment.opens, help: help,
            children: named.map { JumpMenus.terminals(of: $0, selection: selection, fleet: store.fleet) } ?? [],
            // The selection, not `place`, which the detail is handed
            // without its pane.
            target: named.flatMap { Self.backToWorktree($0, from: current, in: store.fleet) },
            terminal: named.flatMap { worktree in
                TerminalCrumb.of(
                    Self.openTerminal(in: worktree, place: current, keyPane: keyPane), in: worktree, selection: selection,
                    fleet: store.fleet)
            })
    }

    /// The terminal open in `worktree` at `place` (ov-267): the one the
    /// place names, else the one the keyboard is in there.
    static func openTerminal(in worktree: Worktree, place: Selection, keyPane: PaneRef?) -> String? {
        switch place {
        case .workspace(_, _, .worktree(worktree.id, let terminal?)?), .looseWorktree(_, worktree.id, let terminal?):
            return terminal
        case .workspace(_, _, .worktree(worktree.id, nil)?), .looseWorktree(_, worktree.id, nil):
            return keyPane.flatMap { $0.worktree == worktree.id ? $0.terminal : nil }
        default:
            return nil
        }
    }

    /// Where the worktree segment's label goes from `place` (ov-267): the
    /// worktree whole, from one of its terminals; nil when it's already
    /// open whole.
    static func backToWorktree(_ worktree: Worktree, from place: Selection, in fleet: Fleet) -> JumpTarget? {
        switch place {
        case .workspace(_, _, .worktree(worktree.id, _?)?), .looseWorktree(_, worktree.id, _?):
            return .go(opening(worktree, terminal: nil, in: fleet))
        default:
            return nil
        }
    }
}
