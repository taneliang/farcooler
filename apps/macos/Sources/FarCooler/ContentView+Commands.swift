import AgentKit
import AppKit
import SwiftUI

/// Routing, behavior, attention and the app and palette commands.
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    // MARK: - Routing

    /// A repository to default the project picker to, when nothing was
    /// chosen yet — the empty state's "New Worktree…" button, ⌘N, and the
    /// palette's "New Worktree…" all reach this with no project
    /// and therefore no host in hand at all, which is the one case where a
    /// default runner is legitimate rather than the picker again in
    /// disguise. This Mac's own repositories come first: it is the runner
    /// guaranteed to be present, the one everything else is optional next to.
    /// Falls back to any repository so the picker still has something to
    /// preselect the very first time, before this Mac has one of its own.
    var defaultProjectID: String? {
        store.repositories.first { $0.host.isEmpty }?.repository.id
            ?? store.repositories.first?.repository.id
    }

    // MARK: - Behavior

    func worktree(host: String, id: String) -> Worktree? {
        store.fleet.worktrees.first { ($0.host ?? "") == host && $0.id == id }
    }

    /// Where the window opens (spec §4.6, ov-182): where it was when the app
    /// last closed, whatever is waiting on Needs You; with nowhere kept, Needs
    /// You while anything is waiting, else the first workspace. See
    /// `SelectionMemory.launch`.
    ///
    /// A place kept is held for its runner, which comes up on its own
    /// schedule, and opened when it has (`WindowRestore`), or at the
    /// nearest level of it that's still there when it's gone. Asked on every
    /// fleet and Needs You change until it has an answer, and never again
    /// after: a window that has opened somewhere, or where somebody already
    /// clicked, isn't moved by a count that rises later.
    func settleLaunch() {
        SelectionMemory.migrate(
            .standard, fleet: store.fleet, ready: { host in store.clients[host]?.hasLoaded ?? true })
        // Not before the window has taken its record (`adoptSession`).
        guard !launched, self.kept != nil else { return }
        guard selection == nil else {
            launched = true
            return
        }
        if let kept = SelectionMemory.kept(
            destination: self.kept?.place?.encoded ?? lastDestination, legacy: lastSelection)
        {
            launched = true
            restoring = DestinationOpen(destination: kept, arrival: .restore, since: Date())
            return
        }
        guard
            let decided = SelectionMemory.launch(
                needsYou: store.needsYou.count, settled: store.needsYouSettled, in: store.fleet)
        else { return }
        launched = true
        selection = decided
    }

    // MARK: - Commands

    /// The terminals of the view on screen, in the order they're drawn:
    /// what ⌘] and ⌘[, ⌥⌘↓ and ⌥⌘↑ and ⌃⌘1… step through (spec §4.9).
    /// Nothing lists every terminal, so stepping through all of them would
    /// walk a list nobody can see.
    var allTerminals: [PaneRef] { Self.stepOrder(shown) }

    /// `allTerminals` for what's shown: column by column, each layout's
    /// panes in tmux's order.
    static func stepOrder(_ shown: [ShownLayout]) -> [PaneRef] {
        shown.flatMap { layout in
            layout.group.terminals.map { PaneRef(host: layout.host, worktree: layout.worktree.id, terminal: $0) }
        }
    }

    /// The pane the keyboard acts on, with its worktree and terminal. See
    /// `WorkspaceScreen.keyPane`.
    var selectedPane: PaneRef? { WorkspaceScreen.keyPane(keyPane, in: shown, selection: selection) }

    var selectedTerminal: (worktree: Worktree, terminal: Terminal)? {
        guard let pane = selectedPane, let worktree = worktree(host: pane.host, id: pane.worktree),
            let terminal = worktree.terminals.first(where: { $0.id == pane.terminal })
        else { return nil }
        return (worktree, terminal)
    }

    /// The layout the detail draws for `ws`, and the layouts its bar offers
    /// beside it. Nil when it draws none.
    ///
    /// What the keyboard's layout commands act on, so ⌃B and a digit counts
    /// the panes on screen and ⌃B n steps through the layouts in the bar —
    /// not through the window tmux calls active, which in the main checkout
    /// can be an orchestrator's. With two columns showing one worktree's
    /// layouts, the one holding the key pane.
    func onScreen(in ws: Worktree) -> (group: PaneGroup, groups: [PaneGroup])? {
        let mine = shown.filter { $0.host == (ws.host ?? "") && $0.worktree.id == ws.id }
        let pick = selectedPane.flatMap { key in mine.first { $0.contains(key) } } ?? mine.last
        return pick.map { ($0.group, $0.groups) }
    }

    // MARK: - Attention

    /// Which terminals the detail pane is actually putting in front of you.
    ///
    /// Not just the selected one. Selecting a pane shows the whole layout it
    /// belongs to, and a terminal tiled beside the one with focus is as much on
    /// screen as the one with focus — reading it took no extra click, so it
    /// cannot go on asking for one.
    ///
    /// Read from `shown`, which is every layout `detail` draws, column by
    /// column. Anything else would have this marking
    /// terminals read that are not on screen, which is the one mistake worse
    /// than the bug it fixes. It used to ask for the layout tmux calls active,
    /// from when `detail` drew that one; since the orchestrators run in the
    /// main checkout's session, that can be Billing's orchestrator while the
    /// checkout's own shells are on screen.
    private var visibleTerminals: [Terminal] {
        shown.flatMap { layout in layout.worktree.terminals.filter { layout.group.terminals.contains($0.id) } }
    }

    /// End `done` for everything on screen, if anyone is there to see it.
    ///
    /// `done` is finished-and-UNSEEN, so it has to end when you see it — and you
    /// see a pane by having it in front of you, not only by clicking on it.
    /// Marking on selection alone missed the commonest case there is: you sit
    /// watching an agent work, it finishes, and because you never had to click
    /// anything the row goes on flagging itself indefinitely. Nothing short of
    /// clicking away and back could clear it.
    ///
    /// Gated on the app being active, which is the whole distinction the feature
    /// rests on. An agent finishing while you are in another app is precisely
    /// what the notification exists for, and a window sitting behind three
    /// others must not quietly mark it read. And, for the marking itself, on
    /// somebody being at the Mac at all (`Presence`): a frontmost window on a
    /// locked or sleeping Mac, or one left for the kitchen, gets fleet events
    /// too, and nobody sees what they bring.
    ///
    /// Only `done`. `blocked` is the agent waiting on an ANSWER, and looking at
    /// a question does not answer it — the daemon agrees, so sending anything
    /// else would only be a subprocess spent to be told no.
    ///
    /// Not routed through `act`: this is a best-effort background
    /// bookkeeping call, not a user-initiated action, and a runner gone quiet
    /// for a moment must not put a banner on screen just because an agent on
    /// it happened to finish.
    ///
    /// It also tells each runner what this window is SHOWING, which is the same
    /// judgement one beat earlier — see `DaemonClient.reportWatching`. Marking
    /// seen ends a `done` that already happened; the claim of attention stops
    /// the notification about it being raised in the first place, and the
    /// difference between the two is the buzz on a wrist about a reply already
    /// on screen. Reported from here rather than from hooks of its own because
    /// there is one question underneath both — "is a person looking at this
    /// pane right now" — and every path that answers it already funnels
    /// through this: a fleet event, coming back to the app, and a selection
    /// change. Anywhere those are wrong about what is on screen, `seen` has
    /// been wrong in the same way for as long as it has existed, and one
    /// answer is the point.
    func markVisibleSeen() {
        guard NSApp.isActive else {
            Notifier.shared.setWatching([], window: windowID)
            return
        }
        // Minimized, or wholly behind other windows: nothing here is on
        // screen, whatever is selected, so it silences no banner and its
        // runners are told so. Asked again when that changes (`body`).
        guard WindowSight.inSight(windowBox.window) else {
            WindowSight.leave(window: windowID, clients: Array(store.clients.values))
            return
        }
        // What `willPresent` asks: the panes on screen, the same set the
        // runners are told below.
        Notifier.shared.setWatching(visibleTerminals.map(\.id), window: windowID)
        let client = selection?.host.flatMap { store.clients[$0] }
        // Full ids, not `short`: resolving an abbreviation costs the CLI a
        // fleet listing, and this runs on a clock. See the `Watching` command
        // in `crates/cli/src/main.rs`.
        client?.reportWatching(visibleTerminals.map(\.id))
        // And every OTHER runner is told it is showing nothing. A window puts
        // exactly one runner's worktree in the detail pane, so switching from a
        // pane on one runner to a pane on another would otherwise leave the
        // first still believing its pane is being watched — silent for as long
        // as the claim takes to age out, on the runner you just walked away
        // from. Every runner, and outside the guard below, because selecting
        // NOTHING is a way of walking away too: `visibleTerminals` is empty
        // then, and so is the claim every runner should be holding.
        for other in store.clients.values where other !== client {
            other.reportWatching([])
        }
        // Only for somebody there, by the same `Presence` the claim above
        // asks — see `DaemonClient.markSeen(onScreen:)`.
        client?.markSeen(onScreen: visibleTerminals)
    }

    // MARK: - Tiling

    /// The worktree a tiling keystroke acts on.
    var tileTarget: Worktree? { currentWorktree }

    /// What the menu bar can act on in this window: each item dimmed when
    /// what it reads here has nothing to act on (ov-211).
    var menuFocus: MainWindowFocus {
        let scene = selection.flatMap(workspaceScene)
        let terminals = allTerminals
        var focus = MainWindowFocus(
            overlayOpen: showQuickCreate || console.console.isOpen, taskOpen: Self.taskOpen(selection),
            hasNavigator: scene?.board != nil)
        focus.sidebarShown = !navigatorHidden
        focus.hasWorktree = currentWorktree != nil
        focus.findsInFile = focusedFiles != nil
        focus.terminals = terminals.count
        focus.stepsTerminals = terminals.count > 1 || (terminals.count == 1 && terminals.first != selectedPane)
        focus.hasAttention =
            NeedsYouNavigation.step(lastOpened: lastAttention, items: store.needsYou, fleet: store.fleet, showing: selection) != nil
        focus.goesBack = jumpBar.history.canGoBack || Self.goesBack(focus: focusColumn, from: selection, trail: trail, board: scene?.board)
        focus.goesForward = jumpBar.history.canGoForward
        focus.history = historyRows
        focus.hasJumpBar = scene?.opened != nil || selection?.focus != nil
        focus.focuses = scene?.opened != nil || selection?.focus != nil
        focus.focused = focusColumn
        focus.inWorkspace = scene != nil
        if let scene, let board = scene.board {
            let entries = worktreeEntries(scene)
            let step = { (by: Int) in
                WorkspaceWorktrees.step(
                    from: selection, by: by, in: entries, host: scene.host, workspace: board, fleet: store.fleet) != nil
            }
            focus.nextWorktree = step(1)
            focus.previousWorktree = step(-1)
        }
        focus.workspaces = WorkspaceNumbers.groups(in: store.fleet).flatMap(\.places).filter { $0.number != nil }.count
        focus.makesWorkspaces = !workspaceRepositories.isEmpty
        focus.layout = tileTarget.map { worktree in
            let screen = onScreen(in: worktree)
            let here = selectedPane.flatMap { screen?.group.pane($0.terminal) } ?? screen?.group.panes.first(where: \.focused)
            // The pane Switch Between Terminal and Chat would switch, found
            // as `tile(_:)` finds it.
            let target = here.flatMap { rect in worktree.terminals.first { $0.id == rect.id } }
                ?? selectedTerminal?.terminal
                ?? screen?.group.panes.first.flatMap { pane in worktree.terminals.first { $0.id == pane.id } }
            return LayoutMenuFocus.make(
                group: screen?.group, here: here, layouts: screen?.groups ?? [],
                switchesMode: target.map { $0.canSwitchPaneMode || $0.isAgentPane } ?? false)
        }
        return focus
    }

    /// Whether Back (⌃⌘←) does anything, by `goBack()`'s own steps: leave
    /// Focus, go up a level, or close a loose worktree to its board.
    nonisolated static func goesBack(focus: Bool, from selection: Selection?, trail: Selection?, board: String?) -> Bool {
        let step = WorkspaceNavigation.backStep(focus: focus, oneAtATime: true, from: selection, trail: trail)
        if step.leavesFocus || step.goesTo != nil { return true }
        guard case .looseWorktree? = selection, let selection else { return false }
        return WorkspaceNavigation.closing(selection, board: board) != nil
    }

    func run(_ command: AppCommand) {
        switch command {
        case .newTerminal:
            // Creates immediately. There is no agent to choose: a terminal is a
            // shell, and whatever you run in it — `claude`, `codex`, a build —
            // is detected from the process, not declared in advance. Asking
            // first was a dialog whose answer was already knowable.
            if let worktree = currentWorktree { newTerminal(in: worktree) }

        case .closeTerminal:
            guard let (worktree, terminal) = selectedTerminal else { return }
            requestClose(terminal, in: worktree)

        case .nextTerminal: step(by: 1)
        case .previousTerminal: step(by: -1)

        case .nextAttention:
            // Straight to whatever is waiting on you, in rank order across
            // every runner. On a fleet of twenty this is the difference
            // between the app being useful and being a list. Walked from the
            // item last opened while the window still shows it, else from the
            // top.
            if let next = NeedsYouNavigation.step(
                lastOpened: lastAttention, items: store.needsYou, fleet: store.fleet, showing: selection)
            {
                open(next.item)
            }

        case .newWorktree:
            // Only reachable with a project registered; the panel has nothing
            // to create into otherwise.
            if store.repositories.isEmpty {
                showAddRepository = true
            } else {
                if lastProject.isEmpty, let id = defaultProjectID { lastProject = id }
                showQuickCreate = true
            }
        case .addRepository: showAddRepository = true
        case .newWorkspace:
            // Only where a runner has workspaces, as the palette's item is.
            if !workspaceRepositories.isEmpty { newWorkspaceName = NewWorkspaceName(name: "") }
        case .showBoard:
            // Only reachable with a project registered; a board needs a
            // repository to be scoped to, the same way New Worktree needs one
            // to create into.
            if store.repositories.isEmpty {
                showAddRepository = true
            } else if case .workspace(let host, let id, nil) = selection {
                // Already there — unless "there" has gone, which is said
                // rather than answered with nothing.
                if board(host: host, id: id) == nil {
                    errorBanner = missingBoardSentence(host: host)
                }
            } else if let target = boardTarget {
                selection = .workspace(host: target.host, workspace: target.workspace.id, focus: nil)
            } else {
                // Never a guess between several. The switcher lists them for
                // exactly this case.
                errorBanner = "Select a workspace first."
            }
        case .back, .forward: step(back: command == .back)
        case .jumpBar: jumpBar.request += 1
        case .focusColumn:
            if selection.flatMap(workspaceScene)?.opened != nil {
                apply(WorkspaceNavigation.boardStep(.toggleFocus, from: boardState))
            } else if selection?.focus != nil {
                focusColumn.toggle()
            }
        case .focusConversation, .focusBoard, .focusTask:
            focusWorkspaceColumn(command)
        case .switchWorkspace: switcherRequest += 1
        case .goToLine: focusedFiles?.goingToLine = true
        case .nextTaskTab: stepTaskTab(by: 1)
        case .previousTaskTab: stepTaskTab(by: -1)
        case .nextWorktree: stepWorktree(by: 1)
        case .previousWorktree: stepWorktree(by: -1)
        case .openInEditor: openInPreferredEditor()
        // Everything the old sidebar's Refresh button read, on every
        // runner: the fleet, and its repositories, roots and layouts, as a
        // reconnection re-reads them (`DaemonClient.onReconnect`).
        case .reload:
            Task {
                for client in store.clients.values {
                    await client.refresh()
                    await client.refreshRepositories()
                    await client.refreshRoots()
                    await client.refreshLayouts()
                }
            }
        case .showShortcuts: showShortcuts = true
        // In a workspace, ⌘F filters its navigator's tasks (ov-103): the
        // find a person in a list of tasks reaches for, bringing the
        // navigator back if it was put away. Anywhere else, the palette,
        // which finds the same workspaces, tasks and agents the old
        // sidebar's search did (ov-178).
        case .search:
            // A file on screen, clicked into: find in it (ov-189).
            if let found = focusedFiles {
                found.finding = true
            // A loose worktree draws its board's navigator too.
            } else if selection.flatMap(workspaceScene)?.board != nil {
                navigatorHidden = false
                boardFilterRequest += 1
            } else {
                console.console.open(recents: true)
            }

        case .showPlan: showPlan()

        case .markAllRead:
            if let scene = selection.flatMap(workspaceScene), let board = scene.board {
                boardStores["\(scene.host)/\(board)"]?.askToMarkAllRead(markReadConfirmation)
            }

        // Toggles rather than opens. ⌘P on an open palette is what a hand
        // reaches for when it changed its mind, and every switcher on this
        // machine closes that way.
        case .commandPalette: console.console.toggle(recents: true)
        // ⌘K: see what's happening, or type to find (ov-214, ov-264).
        case .showActivity: console.console.toggle(recents: false)

        // The window's one sidebar is the navigator (ov-178).
        case .toggleSidebar: toggleNavigator()

        // Moving through a diff belongs to the pane showing one, and only to
        // the FOCUSED one — see `ChangesPane.isFocused`. Listed rather than
        // caught by a `default` so the next command added to `AppCommand`
        // still fails to compile until somebody decides where it goes.
        case .diffNextHunk, .diffPreviousHunk,
            .diffNextFile, .diffPreviousFile,
            .diffNextCommit, .diffPreviousCommit, .diffFirstCommit,
            // Not a movement, but pane-scoped for the same reason: the
            // watermark it moves belongs to the worktree whose diff you were
            // reading, and a window can hold a diff and three terminals.
            .diffMarkRead:
            break
        }
    }

    /// Carry out whatever was chosen in the palette.
    ///
    /// Every case here routes into a method that already existed, and that is
    /// the whole design of `PaletteAction`: the palette knows what you picked
    /// and nothing about what picking it means, so opening a terminal from the
    /// panel and clicking it in the window cannot drift apart.
    func perform(_ action: PaletteAction) {
        console.console.opened()
        switch action {
        case .openTerminal(let worktree, let terminal):
            let host = store.fleet.worktrees.first { $0.id == worktree }?.host ?? ""
            land(on: PaneRef(host: host, worktree: worktree, terminal: terminal))

        case .openWorktree(let id):
            guard let worktree = store.fleet.worktrees.first(where: { $0.id == id }) else { return }
            selection = Self.opening(worktree, terminal: nil, in: store.fleet)

        case .newTerminal(let id):
            guard let worktree = store.fleet.worktrees.first(where: { $0.id == id }) else {
                return
            }
            newTerminal(in: worktree)

        case .openWorkspace(let host, let id):
            selection = .workspace(host: host, workspace: id, focus: nil)

        case .openOrchestrator(let host, let id):
            selection = .workspace(host: host, workspace: id, focus: nil)
            selectOrchestrator(keyboard: .conversation)

        case .openTask(let host, let workspace, let id):
            openTask(id, host: host, workspace: workspace)

        case .openPlan(let host, let workspace, let page):
            openPlan(page, host: host, workspace: workspace)

        case .newWorkspace(let name):
            newWorkspaceName = NewWorkspaceName(name: name)

        case .newWorktree(let described):
            if store.repositories.isEmpty {
                showAddRepository = true
                return
            }
            if lastProject.isEmpty, let id = defaultProjectID { lastProject = id }
            // What was typed into the palette carries over as the description,
            // because in this panel it nearly always was one. It never
            // overwrites a draft already in progress — that draft is often the
            // thing someone opened the palette to go and look something up for.
            if !described.isEmpty, taskDraft.isEmpty { taskDraft = described }
            showQuickCreate = true

        case .togglePaneMode(let worktree, let terminal):
            guard
                let ws = store.fleet.worktrees.first(where: { $0.id == worktree }),
                let target = ws.terminals.first(where: { $0.id == terminal })
            else { return }
            Task { await togglePaneMode(target, in: ws) }

        case .openFile(let worktree, let path):
            guard let ws = store.fleet.worktrees.first(where: { $0.id == worktree }) else { return }
            showInFiles(path, line: nil, in: ws)
        }
    }

    /// Hand the worktree on screen to an editor, from the keyboard.
    ///
    /// The same act as clicking the title bar control, routed through the same
    /// two rules so a click and ⇧⌘E cannot come to different answers: the
    /// editor is whatever `Editors.preferred` says for THIS worktree's runner,
    /// and using it does not change the preference — only picking one out of
    /// the menu does. See `OpenInEditorButton`'s primary action, which this
    /// mirrors deliberately rather than reimplements.
    ///
    /// `refresh()` first, because the menu bar has no `onAppear` to hang it on.
    /// The control gets its probe when it draws; a shortcut can be the first
    /// thing pressed after launch, and without this it would report "no editors
    /// found" on a Mac with four of them installed.
    func openInPreferredEditor(_ chosen: Worktree? = nil) {
        guard let worktree = chosen ?? detailWorktree else {
            errorBanner = "Open a worktree first — there is nothing to hand to an editor."
            return
        }
        let editors = Editors.shared
        editors.refresh()
        let runner = worktree.host ?? ""
        guard let editor = editors.preferred(host: runner) else {
            // Not an error. Nothing is wrong with an app that has never been
            // told which editor you use — so this opens the place you say so,
            // exactly as clicking the control with no editor configured does.
            EditorSettingsLink.open(openSettings)
            return
        }
        Task {
            if let problem = await editors.open(worktree, with: editor) { editorError = problem }
        }
    }
}
