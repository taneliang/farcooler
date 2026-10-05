import AgentKit
import AppKit
import SwiftUI

/// A workspace's detail: the navigator's scene, the conversation, opened places and task views.
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    /// A workspace: the navigator, and what's selected in it in the main
    /// area, the orchestrator or a task or worktree under its jump bar
    /// (ov-92). A loose worktree is drawn here too, beside its repository's
    /// navigator.
    func workspaceDetail(_ scene: WorkspaceScene) -> some View {
        let host = scene.host
        let summary = scene.summary
        let layouts = shown
        let title = sceneTitle(scene, front: layouts.last)
        let plan = splits(scene) ? planViews(scene) : nil
        return WorkspaceView(
            opened: scene.opened,
            hasConversation: scene.hasConversation,
            // Put away with ⌘B, it's drawn as a scene without one.
            hasBoard: scene.board != nil && !navigatorHidden,
            cell: TerminalMetrics.cell(preferences.terminalFont()).width,
            focused: focusColumn,
            navigatorWidth: $navigatorWidth,
            conversation: {
                // Mounted and kept while something else is selected, drawing
                // the same layout; it just doesn't have the keyboard, or
                // count as seen or watched (`shown`).
                let drawn = KeptOrchestrator.conversation(visible: layouts, drawable: drawableLayouts(for: selection))
                conversationColumn(host: host, workspace: summary, shown: drawn.layout, onScreen: drawn.onScreen)
            },
            navigator: {
                if let board = scene.board {
                    boardColumn(
                        host: host, id: board,
                        orchestrator: scene.hasConversation ? navigatorOrchestrator(host: host, workspace: summary) : nil)
                }
            },
            breadcrumb: { place in
                // The path through the one tree (ov-321), Theme › Task ›
                // Lane › Terminal, where it has one; else as before.
                let tree = treeCrumbs(for: place, host: host, workspace: summary, name: workspaceName(host, summary))
                let worktrees = tree == nil ? worktreeCrumb(for: place, scene: scene) : nil
                let crumbs = tree ?? WorkspaceWorktrees.crumbs(
                    crumbs(for: place, host: host, workspace: summary), isHere: worktrees?.isHere == true)
                DrillBreadcrumb(
                    crumbs: crumbs,
                    worktrees: worktrees,
                    onGo: { target in
                        if target == trail { trail = nil }
                        selection = target
                    },
                    onClose: WorkspaceNavigation.closing(place, board: scene.board) == nil ? nil : { closeOpened() },
                    // A task's own worktree keeps the task as the way back, as its Open Worktree does.
                    onOpen: { item in land(NavigationHistory.Stop(item.target, trail: item.trail)) },
                    menus: jumpMenus(crumbs, place: place, host: host, summary: summary),
                    onJump: { jump($0) }, focusRequest: jumpBar.request, onLeave: { keyOpened() },
                    onActive: { jumpBar.active = $0 })
            },
            detail: { place, settled in openedView(place, layouts: layouts, settled: settled) },
            // The canvas beside the orchestrator's chat column (ov-298).
            split: plan != nil,
            home: { plan?.home ?? AnyView(EmptyView()) },
            chatColumns: $chatColumns,
            boardExists: scene.board != nil,
            navigatorFloating: $navigatorFloating,
            strip: { plan?.strip ?? AnyView(EmptyView()) },
            peek: { plan?.peek ?? AnyView(EmptyView()) },
            peeking: $planPeeking
        )
        // Going somewhere puts a peek away.
        .onChange(of: selection) { _, _ in planPeeking = false }
        .onPreferenceChange(WorkspaceWidthPreference.self) { width in
            MainActor.assumeIsolated {
                if let width, width != detailWidth { detailWidth = width }
            }
        }
        // No subtitle: "Billing · shop" is the switcher's, beside it (ov-86).
        .modifier(WindowTitle(title: title.title, subtitle: ""))
    }

    /// The window's title for `scene`: a workspace's, or a loose worktree's
    /// own, as its layout in front says.
    private func sceneTitle(_ scene: WorkspaceScene, front: ShownLayout?) -> (title: String, subtitle: String) {
        if case .looseWorktree(let host, let id, _)? = scene.opened {
            if let front { return Self.frame(of: front, in: store.fleet) }
            let ws = worktree(host: host, id: id)
            return (ws?.windowTitle ?? "Worktree", ws?.windowSubtitle ?? "")
        }
        return workspaceTitle(host: scene.host, workspace: scene.summary, focus: selection?.focus)
    }

    /// The window's title in a workspace (spec §4.9): the workspace, with
    /// "repository · runner" beneath it; a task's key and title with a task
    /// open, and "workspace · repository" beneath.
    private func workspaceTitle(host: String, workspace: WorkspaceSummary?, focus: Focus?)
        -> (title: String, subtitle: String)
    {
        let repository = workspace.flatMap { w in
            store.clients[host]?.repositories.first { $0.id == (w.repository ?? w.id) }?.displayName
        } ?? ""
        let name = workspace.map { $0.isImplicit ? repository : $0.name } ?? "Workspace"
        var leaf: String?
        switch focus {
        case .task(let id)?:
            leaf = taskRow(host: host, workspace: workspace, id: id).map { "\($0.key) \($0.title)" }
        case .worktree(let id, _)?:
            leaf = worktree(host: host, id: id)?.windowTitle
        case .history(let status)?:
            leaf = BoardHistory.title(status)
        case .plan(let page)?:
            leaf = workspace.flatMap { w in store.clients[host].map { boardStore(for: w, client: $0, host: host) } }?
                .plan.title(page)
        case nil:
            break
        }
        return Self.workspaceTitle(
            workspace: name, repository: repository, host: host, leaf: leaf,
            implicit: workspace?.isImplicit == true)
    }

    /// The window's title is the leaf of the breadcrumb: the task or the
    /// worktree opened, with the workspace and repository beneath it
    /// (ov-81 P10). The breadcrumb over the content owns the way back, so the
    /// title no longer repeats the path, and a worktree opened says so here
    /// as a task does instead of leaving the workspace's name up top. In the
    /// workspace itself, the workspace, with "repository · runner" beneath.
    nonisolated static func workspaceTitle(
        workspace: String, repository: String, host: String, leaf: String?, implicit: Bool
    ) -> (title: String, subtitle: String) {
        if let leaf {
            let under = implicit ? [repository] : [workspace, repository]
            return (leaf, under.filter { !$0.isEmpty }.joined(separator: " · "))
        }
        return (workspace, [repository, host].filter { !$0.isEmpty }.joined(separator: " · "))
    }

    /// A task on a workspace's board, as the board last read it.
    private func taskRow(host: String, workspace: WorkspaceSummary?, id: String) -> TaskRow? {
        guard let workspace, let client = store.clients[host] else { return nil }
        return boardStore(for: workspace, client: client, host: host).board.columns
            .flatMap(\.rows).first { $0.id == id }
    }

    /// The conversation column: the orchestrator, drawn as selecting its
    /// row drew it, under its header, or the state around one (spec §8).
    /// See `ConversationColumn`.
    @ViewBuilder
    private func conversationColumn(
        host: String, workspace: WorkspaceSummary?, shown: ShownLayout?, onScreen: Bool = true
    ) -> some View {
        if let workspace {
            let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet)
            let key = "\(host)|\(workspace.id)"
            let canAct = store.refusal(for: host) == nil
            // No header row (ov-214): the orchestrator's state, its harness
            // and its menu are the title bar's status area's.
            VStack(spacing: 0) {
                if let seat {
                    let sharers = WorkspaceScreen.sharers(
                        of: seat, layouts: store.client(for: seat.worktree)?.layouts[seat.worktree.id])
                    ForEach(sharers) { terminal in
                        SharedWindowNotice(title: terminal.label, canAct: canAct) {
                            Task { await moveOutOfOrchestratorWindow(terminal, in: seat.worktree) }
                        }
                    }
                }
                // Ticks only while a start is being timed, in a window
                // somebody can see: the time matters to nothing else (ov-229).
                TimelineView(
                    WhileSchedule(every: 5, running: orchestratorStartedAt[key] != nil && windowVisible)
                ) { context in
                    let state = ConversationColumn.state(
                        seat: seat, isStarting: startingOrchestrators.isStarting(workspace, host: host),
                        startedAt: orchestratorStartedAt[key], now: context.date)
                    conversationBody(
                        state: state, offers: ConversationColumn.offers(state, canAct: canAct),
                        shown: shown, seat: seat, workspace: workspace, host: host, onScreen: onScreen)
                }
            }
            // A start first seen here is timed from here; a live one clears it.
            .task(id: seat.map { "\($0.terminal.id)|\($0.terminal.state)" } ?? "") {
                let starting = seat.map { StateKind.parse($0.terminal.state) == .starting } ?? false
                if starting, orchestratorStartedAt[key] == nil { orchestratorStartedAt[key] = Date() }
                if let seat, StateKind.parse(seat.terminal.state) == .running { orchestratorStartedAt[key] = nil }
            }
        } else {
            ContentUnavailableView {
                Label("Workspace Not Found", systemImage: "square.stack.3d.up")
            } description: {
                Text(missingBoardSentence(host: host))
            }
            .contentCard()
        }
    }

    @ViewBuilder
    private func conversationBody(
        state: ConversationColumn.State, offers: [ConversationColumn.Offer], shown: ShownLayout?,
        seat: BoardPane?, workspace: WorkspaceSummary, host: String, onScreen: Bool
    ) -> some View {
        let placeholder = { (card: Bool) in
            ConversationPlaceholder(
                state: state, offers: offers,
                onStart: { harness in startOrchestrator(workspace, host: host, harness: harness, replace: false) },
                onRestart: { if let seat { Task { await run(.restart, on: seat.terminal, in: seat.worktree) } } },
                onReplace: {
                    let harness = seat.flatMap { OrchestratorHarness(rawValue: Terminal.name(of: $0.terminal.preset)) } ?? .claude
                    orchestratorReplacement = OrchestratorReplacement(host: host, workspace: workspace, harness: harness)
                },
                candidates: OrchestratorAdoption.candidates(for: workspace, host: host, in: store.fleet),
                onUse: { useAsOrchestrator($0) }, carded: card)
        }
        switch state {
        case .live:
            if let shown {
                tiled(shown, titled: false, keyboard: onScreen)
            } else if let seat {
                bareTerminal(seat, keyboard: onScreen)
            }
        case .lost:
            // Its last screen, dimmed, under what can be done about it.
            if let shown {
                ZStack {
                    tiled(shown, titled: false).opacity(0.35).allowsHitTesting(false)
                    // style-exempt: a scrim over the lost terminal's dimmed last screen
                    placeholder(false).background(.regularMaterial, in: .card).paneCanvas()
                }
            } else {
                placeholder(true)
            }
        case .none, .starting:
            placeholder(true)
        }
    }

    /// The orchestrator's row in the navigator (ov-92): its state, what
    /// it's doing now, and, with none, the ways to start one.
    func navigatorOrchestrator(host: String, workspace: WorkspaceSummary?) -> NavigatorOrchestrator? {
        guard let workspace else { return nil }
        let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet)
        let canAct = store.refusal(for: host) == nil
        let column = ConversationColumn.state(
            seat: seat, isStarting: startingOrchestrators.isStarting(workspace, host: host),
            startedAt: orchestratorStartedAt["\(host)|\(workspace.id)"], now: Date())
        // Asked for here and not yet seated: starting, as its column says.
        var state = OrchestratorRow.state(seat: seat)
        if case .starting = column { state = .starting }
        return NavigatorOrchestrator(
            state: state,
            agent: OrchestratorMenu.agentName(seat),
            status: seat?.terminal.status,
            nowDoing: OrchestratorRow.nowDoing(seat?.terminal, state: state),
            offers: ConversationColumn.offers(column, canAct: canAct),
            candidates: OrchestratorAdoption.candidates(for: workspace, host: host, in: store.fleet),
            onSelect: { selectOrchestrator(keyboard: .board) },
            onStart: { harness in startOrchestrator(workspace, host: host, harness: harness, replace: false) },
            onUse: { useAsOrchestrator($0) })
    }

    /// The workspace's name as its crumb says it: a repository's implicit
    /// one by the repository's.
    private func workspaceName(_ host: String, _ workspace: WorkspaceSummary?) -> String {
        let repository = workspace.flatMap { w in
            store.clients[host]?.repositories.first { $0.id == (w.repository ?? w.id) }?.displayName
        } ?? ""
        return workspace.map { $0.isImplicit ? repository : $0.name } ?? "Workspace"
    }

    /// The breadcrumb over what's opened: Workspace › Task, Workspace › Task
    /// › Worktree, or Workspace › Worktree; for a loose worktree, its board's
    /// workspace › the worktree. `place` is what's drawn, which may be the
    /// one leaving rather than the window's.
    private func crumbs(for place: Selection, host: String, workspace: WorkspaceSummary?) -> [WorkspaceNavigation.Crumb] {
        let name = workspaceName(host, workspace)
        if case .looseWorktree(_, let id, _) = place {
            let here = WorkspaceNavigation.Crumb(title: worktree(host: host, id: id)?.task ?? "Worktree", target: nil)
            guard let workspace else { return [here] }
            return [.init(title: name, target: .workspace(host: host, workspace: workspace.id, focus: nil)), here]
        }
        // The window's own, with its trail, while it's the one drawn.
        let current = WorkspaceSelection.samePlace(place, selection) ? selection : place
        return WorkspaceNavigation.crumbs(
            current, trail: current == selection ? trail : nil,
            workspace: name,
            task: { id in
                taskRow(host: host, workspace: workspace, id: id).map { "\($0.key) \($0.title)" } ?? "Task"
            },
            worktree: { id in worktree(host: host, id: id)?.task ?? "Worktree" },
            projectTerminal: { id, terminal in
                ProjectTerminals.name(of: terminal, in: worktree(host: host, id: id), fleet: store.fleet)
            },
            plan: { page in
                workspace.flatMap { w in store.clients[host].map { boardStore(for: w, client: $0, host: host) } }?
                    .plan.title(page) ?? page.word
            })
    }

    /// What's opened beside the board: a task, or a worktree opened whole.
    ///
    /// `place` is the window's own while it's open, drawn with the layouts on
    /// screen and the keyboard; or the one leaving, sliding out or fading
    /// under the next, drawn with its layout and no keyboard, so its
    /// terminal view stays what it was until it's gone.
    @ViewBuilder
    private func openedView(_ place: Selection, layouts: [ShownLayout], settled: Bool) -> some View {
        let current = WorkspaceSelection.samePlace(place, selection)
        let shown =
            current
            ? layouts.last { $0.column != .conversation }
            : drawableLayouts(for: place).last { $0.column != .conversation }
        switch place {
        case .workspace(let host, _, .task(let id)?):
            // Its agent's layout whichever tab is in front, so the
            // terminal is one view across them; whether it's on screen is
            // the tab's to say (`WorkspaceScreen.visible`).
            taskView(
                host: host, id: id, place: place,
                shown: drawableLayouts(for: place).last { $0.column != .conversation }, keyboard: current,
                settled: settled
            )
            // The task is one card on the plane (ov-223): its header, tabs and
            // body together, in the paper's color, inset by the window's gutter.
            .contentCard()
        case .workspace(let host, let id, .history(let status)?):
            if let client = store.clients[host], let workspace = board(host: host, id: id) {
                BoardHistoryView(
                    store: boardStore(for: workspace, client: client, host: host), status: status,
                    onOpen: { row in chooseTask(row.id, host: host, workspace: id, glance: false) }
                )
            } else {
                ContentUnavailableView("Board Not Found", systemImage: "checklist")
                    .contentCard()
            }
        case .workspace(let host, let id, .plan(.needsYou)?):
            if let client = store.clients[host], let workspace = board(host: host, id: id) {
                PlanNeedsYouPage(
                    needsYou: planNeedsYou(host: host, workspace: workspace),
                    board: boardStore(for: workspace, client: client, host: host),
                    onOpenTheme: { openPlan(.theme($0), host: host, workspace: id) },
                    onOpenTask: { chooseTask($0, host: host, workspace: id, glance: false) })
            } else {
                ContentUnavailableView("Board Not Found", systemImage: "flag").contentCard()
            }
        case .workspace(let host, let id, .plan(let page)?):
            if let client = store.clients[host], let workspace = board(host: host, id: id) {
                let board = boardStore(for: workspace, client: client, host: host)
                PlanPageView(plan: board.plan, page: page, context: planContext(board, host: host, workspace: id))
            } else {
                ContentUnavailableView("Board Not Found", systemImage: "map")
                    .contentCard()
            }
        case .workspace(let host, _, .worktree(let wt, _)?), .looseWorktree(let host, let wt, _):
            if !settled {
                // Passed on the way: its terminals wait until it settles.
                Color.clear
            } else if let paneless = WorkspaceScreen.paneless(current ? selection : place, in: store.fleet, shown: shown) {
                // A lost terminal clicked on its card: its own page, with
                // Restart and Dismiss, not the card again (ov-191). Asked of
                // the selection, not `place`, which leaves the pane out
                // (`WorkspaceSelection.place`): asked of that, this never
                // fired, and the click still did nothing.
                bareTerminal(paneless, keyboard: current)
            } else if let shown {
                tiled(shown, titled: false, keyboard: current)
            } else if let ws = worktree(host: host, id: wt) {
                worktreeDetail(ws)
            } else {
                ContentUnavailableView("Worktree Not Found", systemImage: "folder")
                    .contentCard()
            }
        default:
            EmptyView()
        }
    }

    /// Back: up one level, Focus first. ⌃⌘← goes along the breadcrumb to a
    /// task a worktree was opened from; Esc (`toOrchestrator`) goes up to
    /// the orchestrator (ov-92).
    func goBack(toOrchestrator: Bool = false) {
        let step = WorkspaceNavigation.backStep(
            focus: focusColumn, oneAtATime: true, toOrchestrator: toOrchestrator, from: selection, trail: trail)
        if step.leavesFocus { focusColumn = false }
        if let back = step.goesTo {
            guard back.focus != nil else {
                closeOpened()
                return
            }
            if back == trail { trail = nil }
            // The keyboard follows to the level it lands on, by the
            // selection's own rule (`WorkspaceScreen.keyPane`), or, with no
            // terminal there, to the view itself.
            selection = back
            if WorkspaceScreen.keyPane(nil, in: shownLayouts(for: back), selection: back) == nil {
                windowBox.window?.makeFirstResponder(nil)
            }
        } else if step.leavesFocus {
            keyOpened()
        } else if case .looseWorktree? = selection {
            closeOpened()
        }
    }

    /// Close what's opened: the jump bar's close button, a click on the
    /// selected task, or Back from a task. The orchestrator is selected
    /// again, and the navigator keeps the keyboard, so ↑ and ↓ go on from
    /// it.
    private func closeOpened() {
        let board = selection.flatMap(workspaceScene)?.board
        guard let current = selection, let closed = WorkspaceNavigation.closing(current, board: board) else { return }
        trail = nil
        let step = WorkspaceNavigation.boardStep(.close, from: boardState)
        if step.keyboard == .board { boardKeyboardPending = true }
        focusColumn = step.focus
        selection = closed
        key(step.keyboard)
    }

    /// The orchestrator selected (⌥⌘1, its row, ↑ or ↓ onto it): whatever
    /// was open goes, and `keyboard` says where the keyboard goes, into the
    /// orchestrator or staying on the navigator.
    func selectOrchestrator(keyboard: WorkspaceNavigation.KeyTarget) {
        guard let current = selection, let scene = workspaceScene(current) else { return }
        trail = nil
        focusColumn = false
        if let board = scene.board, current.focus != nil || scene.opened != nil {
            if keyboard == .board { boardKeyboardPending = true }
            selection = .workspace(host: scene.host, workspace: board, focus: nil)
        }
        key(keyboard)
    }

    /// The keyboard to what the main area shows: what's opened, else the
    /// orchestrator.
    func keyMain() {
        let layouts = shownLayouts(for: selection)
        if let pane = (layouts.last(where: { $0.column != .conversation }) ?? layouts.first)
            .flatMap(WorkspaceScreen.columnPane)
        {
            step(to: pane)
        } else {
            keyboardOnBoard = false
            keyPane = nil
            windowBox.window?.makeFirstResponder(nil)
        }
    }

    /// The keyboard to the task's or worktree's terminal, if it has one on
    /// screen, else to the view itself, so Esc and the arrows reach it.
    func keyOpened() {
        if let pane = shownLayouts(for: selection).last(where: { $0.column != .conversation })
            .flatMap(WorkspaceScreen.columnPane)
        {
            step(to: pane)
        } else if case .workspace(_, _, nil)? = selection {
            // Nothing opened: the orchestrator takes it.
            keyMain()
        } else {
            keyPane = nil
            windowBox.window?.makeFirstResponder(nil)
        }
    }

    /// A task, beside the navigator (spec §4.4, ov-98): its header, and
    /// under it its three tabs, Overview, Agent and Changes.
    @ViewBuilder
    private func taskView(
        host: String, id: String, place: Selection, shown: ShownLayout?, keyboard: Bool, settled: Bool
    ) -> some View {
        let summary = place.workspace.flatMap {
            WorkspaceScreen.workspace($0, host: host, in: store.fleet, repositories: store.clients[host]?.repositories.map(\.id) ?? [])
        }
        if let client = store.clients[host], let summary {
            let board = boardStore(for: summary, client: client, host: host)
            if let row = board.board.columns.flatMap(\.rows).first(where: { $0.id == id }) {
                taskView(
                    row: row, board: board, client: client, host: host, shown: shown, keyboard: keyboard,
                    settled: settled)
            } else if !board.hasRead {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .task(id: ObjectIdentifier(board)) { await board.readIfNeverRead() }
            } else {
                ContentUnavailableView {
                    Label("Task Not on Board", systemImage: "checklist")
                } actions: {
                    Button("Close") { closeOpened() }
                }
            }
        } else {
            ContentUnavailableView("Task Not Found", systemImage: "checklist")
        }
    }

    /// Ask the Orchestrator for `workspace`'s tasks: on while it has an
    /// orchestrator running, which it reads from the fleet as it is now. It
    /// leaves the draft in that composer, or pastes it into a terminal
    /// orchestrator (`AskOrchestrator.deliver`), and goes there; it starts
    /// nothing.
    func askOrchestrator(host: String, workspace: WorkspaceSummary) -> AskOrchestrator.Action {
        // As the runner lists it now: the board's own copy is kept across an
        // orchestrator starting or stopping.
        let live = WorkspaceScreen.workspace(
            workspace.id, host: host, in: store.fleet,
            repositories: store.clients[host]?.repositories.map(\.id) ?? []) ?? workspace
        let seat = WorkspaceScreen.orchestrator(of: live, host: host, in: store.fleet)
        return AskOrchestrator.Action(
            available: seat != nil,
            perform: { row in
                guard let seat else { return }
                selection = .workspace(host: host, workspace: workspace.id, focus: nil)
                let client = store.client(for: seat.worktree)
                Task { @MainActor in
                    let delivery = await AskOrchestrator.deliver(
                        row, to: seat,
                        paste: { text in await client?.draftPrompt(terminal: seat.terminal.short, text: text) ?? false },
                        copy: client?.copyToClipboard ?? AskOrchestrator.copyToPasteboard)
                    // A terminal pane has no composer to fill: the person
                    // pastes, so the pane takes the keyboard.
                    if delivery != .composer {
                        focus(PaneRef(host: host, worktree: seat.worktree.id, terminal: seat.terminal.id))
                    }
                    if delivery == .copied { errorBanner = AskOrchestrator.copiedNotice(for: row) }
                }
            })
    }

    private func taskView(
        row: TaskRow, board: TaskBoardStore, client: DaemonClient, host: String, shown: ShownLayout?,
        keyboard: Bool = true, settled: Bool = true
    ) -> some View {
        let onRunner = boardAgents(host: host, client: client)
        let agents = WorkspaceScreen.agents(of: row.id, host: host, in: store.fleet)
        let chosen = WorkspaceScreen.agent(of: row.id, host: host, in: store.fleet, chosen: chosenAgents[row.id])
        let worktreeID = TaskColumnModel.worktree(of: row, agent: chosen)
        let lane = worktreeID.flatMap { worktree(host: host, id: $0) }
        let agent = TaskColumnModel.agent(
            hasAgent: chosen != nil, worktree: lane?.id,
            stopped: TaskColumnModel.hadAgent(row.id, host: host, in: store.fleet))
        let showsChanges = lane != nil && client.changesSupported != false
        let tab = taskTabs.tab(for: row.id, agentWorking: chosen != nil)
        let openWorktree = {
            guard let lane, let current = selection else { return }
            let opened = WorkspaceNavigation.openWorktree(lane.id, from: current)
            trail = opened.trail
            trailWorktree = lane.id
            selection = opened.next
        }
        let ask = askOrchestrator(host: host, workspace: board.workspace)
        // Open Terminal in This Task's Worktree, where there is one and its
        // runner answers.
        let newTerminalHere = TaskColumnModel.newTerminalAction(
            in: lane, refused: store.refusal(for: host) != nil,
            openWorktree: openWorktree, newTerminal: { newTerminal(in: $0) })
        let start = TaskStartPanel(
            sentence: TaskColumnModel.sentence(agent) ?? "", row: row, ask: ask)
        return VStack(spacing: 0) {
            TaskViewHeader(row: row, ask: ask, agent: chosen)
            PlanTaskLineView(plan: board.plan, task: row.id) { openPlan($0, host: host, workspace: board.workspace.id) }
            TaskTabBar(
                tab: tab, onChoose: { choose($0, for: row.id) },
                worktree: lane.map { WorkspaceScreen.ownTerminals(of: $0, fleet: store.fleet) },
                agents: agents, chosen: chosen,
                onChooseAgent: { pane in chosenAgents[row.id] = pane.terminal.id },
                onOpenWorktree: openWorktree,
                onNewTerminal: newTerminalHere)
            // The task's own terminals, a dev server among them: in view over
            // every tab, each with Close. Nothing here closes on its own.
            let terminals = TaskTerminals.terminals(of: row, host: host, in: store.fleet)
            if let lane, !terminals.isEmpty {
                TaskTerminalsStrip(
                    terminals: terminals, onThisMac: host.isEmpty,
                    onOpen: { terminal in
                        openWorktree()
                        open(lane, terminal: terminal.id)
                    },
                    onOpenInBrowser: { terminal in Task { await run(.openInBrowser, on: terminal, in: lane) } },
                    onRename: store.clients[host]?.daemonBuild?.can(.terminalNames) == true
                        ? { terminal in Task { await run(.rename, on: terminal, in: lane) } } : nil,
                    onClose: { terminal in Task { await run(.close, on: terminal, in: lane) } })
            }
            TaskTabs(tab: tab) {
                ScrollView {
                    TaskColumnCard(
                        row: row, store: board, orchestrator: onRunner.orchestrator(for: row),
                        speaksOfAgents: onRunner.runnerRecordsTasks, onGoTo: { go(to: $0) }
                    )
                    .padding(TaskTypography.inset)
                }
                // The record runs past the window on a long task: bars that
                // stay, and a soft edge that says there is more below.
                .scrollIndicators(.visible)
                .scrollEdgeEffectStyle(.soft, for: .bottom)
                .background(WorkspaceStyle.paper)
            } agent: {
                // A task passed on the way, glancing: no terminal mounted
                // until it settles. Mounted once, then kept behind the other
                // tabs (`TaskTabs`); on screen and given the keyboard only
                // in front.
                switch TaskColumnModel.agentView(
                    hasAgent: chosen != nil, settled: settled, hasLayout: shown != nil,
                    layoutsRead: chosen.map { client.layouts[$0.worktree.id] != nil } ?? false)
                {
                case .start: start
                case .waiting: Color.clear
                case .tiled: if let shown { tiled(shown, titled: false, keyboard: keyboard && tab == .agent) }
                // Working the task, and in no layout read yet.
                case .bare: if let chosen { bareTerminal(chosen, keyboard: keyboard && tab == .agent) }
                }
            } changes: {
                if let lane, showsChanges {
                    if settled {
                        TaskColumnChanges(
                            changes: changesStore(for: lane, client: client),
                            isFocused: TaskColumnModel.changesFocused(focus: changesFocus, task: row.id, tab: tab),
                            agents: lane.reviewAgentTargets(), onFocus: { changesFocus = row.id })
                    }
                } else if lane != nil {
                    TaskStartPanel(sentence: "Update Far Cooler on this runner to see changes here.")
                } else {
                    start
                }
            } files: {
                // Its worktree's files, read-only (ov-189).
                if let lane, client.showsFiles != false {
                    if settled {
                        FilesPane(model: files.model(for: lane, client: client), onFocus: { files.focus = lane.id })
                    }
                } else if lane != nil {
                    TaskStartPanel(sentence: "Update Far Cooler on this runner to see its files here.")
                } else {
                    start
                }
            }
        }
        // The card's record and question, read for the task on screen once
        // it has settled: one read for a held arrow's whole walk.
        .task(id: settled ? row.id : nil) {
            if settled { await board.open(row) }
        }
    }

    /// Move to Its Own Window: `terminal`, sharing the orchestrator's
    /// window, gets a window of its own (`layout break`, tmux's
    /// `break-pane -d`), so the orchestrator keeps its window and focus.
    /// Only on this click: nothing rearranges a runner's windows unasked.
    @discardableResult
    func moveOutOfOrchestratorWindow(_ terminal: Terminal, in worktree: Worktree) async -> Bool {
        guard let client = store.client(for: worktree) else { return false }
        if !(await client.moveToOwnWindow(terminal, in: worktree)) {
            errorBanner = "Couldn’t move \(terminal.label) to its own window. Check that the runner is reachable, then try again."
            return false
        }
        return true
    }

    /// A worktree with no layout yet: its card of terminals.
    ///
    /// Without the orchestrators seated in it, which are their workspaces'
    /// conversation columns', and with where to find them instead.
    private func worktreeDetail(_ ws: Worktree) -> some View {
        let host = ws.host ?? ""
        return WorktreeDetail(
            worktree: WorkspaceScreen.ownTerminals(of: ws, fleet: store.fleet),
            hosted: WorkspaceScreen.seated(in: ws, fleet: store.fleet).map { seat in
                WorktreeDetail.Hosted(name: seat.workspace.name) {
                    selection = .workspace(host: host, workspace: seat.workspace.id, focus: nil)
                }
            },
            onNewTerminal: { newTerminal(in: ws) },
            onHide: { Task { await act(.hide, on: ws) { c in await c.hideWorktree(ws.short) } } },
            onUnhide: { Task { await act(.unhide, on: ws) { c in await c.unhideWorktree(ws.short) } } },
            onRemove: { removeWorktree = ws },
            onOpenTerminal: { t in open(ws, terminal: t.id) },
            onTerminalAction: { action, t in Task { await run(action, on: t, in: ws) } },
            canRename: store.clients[host]?.daemonBuild?.can(.terminalNames) == true,
            onThisMac: host.isEmpty,
            onShowChanges: showChangesAction(for: ws, usable: store.refusal(for: host) == nil)
        )
    }

    /// Open `worktree`, or `terminal` in it, as its navigator row and its card
    /// do. See `navigate(to:key:)`.
    func open(_ worktree: Worktree, terminal: String?) {
        navigate(to: Self.opening(worktree, terminal: terminal, in: store.fleet))
    }

    /// Go to `next`, an explicit open: a navigator row, a card, Needs You, the
    /// palette, or a pane gone to from the keyboard. A terminal it names that
    /// shares a seated orchestrator's window is moved to a window of its own
    /// first (`moveOutOfOrchestratorWindow`: the sharer, never the
    /// orchestrator), because the Orchestrator column draws that window and
    /// the checkout can't draw a pane apart from it (ov-78). Opening it is
    /// the ask. If the move fails the selection stays where it was, with the
    /// banner saying why, rather than landing on a pane that isn't there.
    func navigate(to next: Selection, key: PaneRef? = nil) {
        func land() {
            selection = next
            if let key { keyPane = key }
        }
        guard let named = WorkspaceScreen.namedTerminal(next),
            let listed = worktree(host: named.host, id: named.worktree),
            let sharer = listed.terminals.first(where: { $0.id == named.terminal }),
            WorkspaceScreen.seat(
                sharedBy: sharer.id, in: listed, fleet: store.fleet,
                layouts: store.client(for: listed)?.layouts[listed.id]) != nil
        else {
            land()
            return
        }
        Task {
            if await moveOutOfOrchestratorWindow(sharer, in: listed) { land() }
        }
    }
}
