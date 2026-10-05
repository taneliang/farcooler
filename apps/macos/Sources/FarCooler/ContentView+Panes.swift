import AgentKit
import AppKit
import SwiftUI

/// Panes drawn in the detail area: bare terminals, the board column and tiled layouts.
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    /// A pane drawn on its own, for the moment before its layout is read, or
    /// a pane in no layout at all: "we haven't read the layouts yet" and
    /// "it's in none" look the same from here, and showing the terminal is
    /// the right answer to both.
    func bareTerminal(_ pane: BoardPane, keyboard: Bool = true) -> some View {
        let ws = pane.worktree
        let term = pane.terminal
        let client = store.client(for: ws)
        return TerminalPane(
            terminal: term,
            worktree: ws,
            binary: client?.cliPath,
            environment: client?.cliEnvironment ?? [:],
            hostArguments: client?.cliHostArguments ?? [],
            linkGeneration: client?.linkGeneration ?? 0,
            refusal: { store.refusal(for: ws) },
            onGeometry: { cols, rows in
                // Not through `act`: this is geometry, not a click.
                await store.client(for: ws)?.resize(terminal: term.short, columns: cols, rows: rows)
            },
            onSearchFiles: { query in
                await store.client(for: ws)?.searchFiles(in: ws, query: query) ?? []
            },
            onAction: { action in Task { await run(action, on: term, in: ws) } },
            hasKeyboard: keyboard && WorkspaceScreen.bareTakesKeyboard(shown)
        )
        // A path its agent's tool call names opens in this worktree's Files.
        .environment(\.filesWorktree, ws)
    }

    /// A workspace's navigator, or a sentence saying where its board went.
    @ViewBuilder
    func boardColumn(host: String, id: String, orchestrator: NavigatorOrchestrator?) -> some View {
        if let client = store.clients[host], client.daemonBuild.map({ !$0.can(.tasks) }) == true {
            // A runner too old for boards: said, rather than a board that
            // can't be read.
            ContentUnavailableView {
                Label("No Board on This Runner", systemImage: "checklist")
            } description: {
                Text("This runner’s Far Cooler is too old for boards. Update it there to see this workspace’s tasks.")
            }
        } else if let client = store.clients[host], let workspace = board(host: host, id: id),
            let repository = client.repositories.first(where: {
                $0.id == (workspace.repository ?? workspace.id)
            })
        {
            TaskBoardView(
                store: boardStore(for: workspace, client: client, host: host),
                client: client,
                agents: boardAgents(host: host, client: client),
                onGoTo: { pane in go(to: pane) },
                selected: WorkspaceNavigation.selectedTask(selection, trail: trail, board: id),
                focusRequest: boardFocusRequest,
                onKeyboard: { keyboardOnBoard = true },
                onEnter: { focusWorkspaceColumn(.focusTask) },
                hasKeyboard: keyboardOnBoard,
                worktrees: { board in boardWorktrees(host: host, workspace: workspace, client: client, board: board) },
                orchestrator: orchestrator,
                current: Navigator.current(
                    selection, trail: trail, board: id,
                    terminals: ProjectTerminals.ids(for: workspace, host: host, in: store.fleet)),
                onStep: { item in step(to: item, host: host, workspace: workspace) },
                onHistory: { status in openHistory(status, host: host, workspace: workspace.id) },
                filterRequest: boardFilterRequest,
                ask: askOrchestrator(host: host, workspace: workspace),
                split: $navigatorSplit,
                planPage: planPage(host: host, workspace: workspace.id),
                onPlan: { page in openPlan(page, host: host, workspace: workspace.id) }
            )
        } else {
            // Said, rather than the generic "Select a worktree": this
            // was a board, and the reader should know where it went.
            ContentUnavailableView {
                Label("Board Not Found", systemImage: "checklist")
            } description: {
                Text(missingBoardSentence(host: host))
            }
        }
    }

    /// One shown layout, wired up.
    ///
    /// One builder for every column. Selecting a pane and selecting the
    /// worktree it is in put the same view on screen with the same six
    /// callbacks, and while they were written out twice they drifted: the drag
    /// handler was fixed in one copy and not the other, so dropping a pane
    /// behaved differently depending on which row you had clicked last.
    @ViewBuilder
    func tiled(_ shown: ShownLayout, titled: Bool = true, keyboard: Bool = true) -> some View {
        let ws = shown.worktree
        if let client = store.client(for: ws) {
            tiled(
                shown, client: client, frame: Self.frame(of: shown, in: store.fleet), titled: titled,
                keyboard: keyboard)
        } else {
            placeholder
        }
    }

    func tiled(
        _ shown: ShownLayout, client: DaemonClient, frame: (title: String, subtitle: String), titled: Bool,
        keyboard: Bool = true
    ) -> some View {
        let ws = shown.worktree
        return TileView(
            groups: shown.groups,
            showing: shown.group.id,
            worktree: ws,
            changes: changesStore(for: ws, client: client),
            binary: client.cliPath,
            environment: client.cliEnvironment,
            hostArguments: client.cliHostArguments,
            linkGeneration: client.linkGeneration,
            refusal: { store.refusal(for: ws) },
            onFocus: { id in
                focus(PaneRef(host: ws.host ?? "", worktree: ws.id, terminal: id))
                guard let pane = store.client(for: ws)?.group(holding: id, in: ws.id)?.pane(id)
                else { return }
                Task { await act(.arrange, on: ws) { c in await c.focusPane(pane.short, in: ws) } }
            },
            onSelectGroup: { chosen in
                Task {
                    // Land in the layout you just chose, not on whatever pane the
                    // previous one had focused.
                    let groups = await act(.arrange, on: ws, default: []) { c in
                        await c.selectLayout(chosen.id, in: ws)
                    }
                    reveal(groups, in: ws)
                }
            },
            onDropOnPane: { dragged, target, side in
                placePane(dragged, onto: target, side: side, in: ws)
            },
            onViewport: { layout, columns, rows in
                // Not routed through `act` — see `onGeometry`'s
                // comment above: this fires from pane geometry, not a click.
                //
                // The layout drawn, by name: tmux's active window can be an
                // orchestrator's while the checkout's own is on screen.
                await store.client(for: ws)?.viewport(
                    columns: columns, rows: rows, in: ws, layout: layout)
            },
            onResizeDivider: { terminal, side, cells in
                resizeDivider(terminal, side: side, cells: cells, in: ws)
            },
            onSearchFiles: { query in await store.client(for: ws)?.searchFiles(in: ws, query: query) ?? [] },
            onSwitchPaneMode: { terminal in Task { await togglePaneMode(terminal, in: ws) } },
            onTerminalAction: { action, terminal in Task { await run(action, on: terminal, in: ws) } },
            title: frame.title,
            subtitle: frame.subtitle,
            setsTitle: titled,
            hasKeyboard: KeptOrchestrator.takesKeyboard(
                shown, onScreen: keyboard, key: selectedPane, onBoard: keyboardOnBoard)
        )
        .environment(\.filesWorktree, ws)
    }

    /// The runner this window's restore is waiting for, while it is
    /// (`DestinationOpener.waitsForRunner`, ov-279).
    var restoringRunner: String? {
        guard let restoring, DestinationOpener.waitsForRunner(restoring, in: MacDestination.world(of: store))
        else { return nil }
        return restoring.destination.runner.host
    }

    /// The detail with no workspace to show: the fleet's own state first,
    /// before it has anything in it (`FleetPlaceholder`), then "choose one".
    /// While a restore is pending, read again at the moments its phase can
    /// change: `FleetPlaceholder.connectingDelay`, when a runner still not up
    /// is said to be connecting, and `connectingLimit`, when that turns to
    /// unreachable. Wakes at those times, not a clock (`IdleCostTests`).
    @ViewBuilder var placeholder: some View {
        if let since = restoring?.since {
            let runner = restoringRunner
            TimelineView(.explicit(FleetPlaceholder.wakes(since: since))) { context in
                placeholder(restoringOn: runner, waited: context.date.timeIntervalSince(since), restoring: true)
            }
        } else {
            placeholder(restoringOn: nil, waited: 0, restoring: false)
        }
    }

    /// What a runner a restore waits for has said is wrong with it: this
    /// Mac's daemon's error, or a runner's refusal.
    func trouble(on runner: String) -> String? {
        let client = store.clients[runner]
        return (runner.isEmpty ? client?.fleetError : nil) ?? client?.state.refusal
    }

    private func placeholder(restoringOn runner: String?, waited: TimeInterval, restoring: Bool) -> some View {
        let local = store.clients[""]
        return FleetPlaceholder(
            phase: FleetPlaceholder.phase(
                hasWorktrees: !store.fleet.worktrees.isEmpty, localLoaded: local?.hasLoaded == true,
                localError: local?.fleetError, hasRepositories: !store.repositories.isEmpty,
                restoringOn: runner, trouble: runner.flatMap(trouble(on:)), waited: waited, restoring: restoring),
            onShowNeedsYou: { selection = .needsYou },
            onOpenMain: FleetPlaceholder.mainToOpen(in: store.repositories, fleet: store.fleet).map { target in
                { selection = .workspace(host: target.host, workspace: target.workspace.id, focus: nil) }
            },
            onNewWorkspace: workspaceRepositories.isEmpty ? nil : { newWorkspaceName = NewWorkspaceName(name: "") },
            onAddRepository: { showAddRepository = true },
            onNewWorktree: { newWorktreeIntent = NewWorktreeIntent() },
            // The runner a restore waits for, dialed again; else this Mac's read.
            onTryAgain: {
                if let runner, !runner.isEmpty { store.reconnect(runner) } else { Task { await local?.refresh() } }
            })
    }
}
