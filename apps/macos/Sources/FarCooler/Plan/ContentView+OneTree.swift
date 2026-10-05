import AgentKit
import AppKit
import SwiftUI

// The window's side of the one tree (ov-321): what the tree is built from,
// which node the selection is, where a node goes, the jump bar's path through
// it and ⌘↑. The rules are AgentKit's `OneTree`.

extension ContentView {
    /// The navigator's tree for `workspace`, or nil while the navigator
    /// draws the Board view (View ▸ Tasks by Status).
    func oneTreeSidebar(host: String, workspace: WorkspaceSummary, client: DaemonClient) -> OneTreeSidebar? {
        guard !showsBoardList else { return nil }
        let board = boardStore(for: workspace, client: client, host: host)
        return OneTreeSidebar(
            tree: { filter in
                board.plan.oneTree(
                    oneTreeInput(host: host, workspace: workspace, client: client, filter: filter))
            },
            selected: treeTarget(selection, workspace: workspace.id),
            hint: treeHint, key: "\(host)|\(workspace.id)",
            onChoose: { node in
                treeHint = node.id
                if let target = node.target { goInTree(target, host: host, workspace: workspace) }
            },
            settled: { board.hasRead && (board.plan.hasRead || !board.plan.available) },
            menu: { node in AnyView(treeMenu(node, host: host, workspace: workspace, client: client)) },
            fold: treeFold)
    }

    /// The workspace's Needs You count: the title bar's and the sidebar's
    /// one number (review H3), from `WorkspaceNeedsYou.count`. `columnCount`
    /// is the board's Needs Decision column as the caller read it.
    func needsYouCount(host: String, workspace: WorkspaceSummary, client: DaemonClient, columnCount: Int) -> Int {
        let board = boardStore(for: workspace, client: client, host: host)
        return WorkspaceNeedsYou.count(
            items: WorkspaceCounts.count(for: workspace, host: host, in: store.needsYou), columnCount: columnCount,
            listRead: client.needsYouLoaded, listServed: client.servesNeedsYou,
            themeAsks: board.plan.plan.shownThemes.filter { !$0.ownerAsk.isEmpty }.count)
    }

    /// A row's context menu (review M2): a card's, as the Board view's row
    /// has it; a lane's or worktree's, the sidebar row's (New Terminal and
    /// Unhide among them); a terminal's; New Terminal in the checkout.
    @ViewBuilder
    func treeMenu(_ node: OneTreeNode, host: String, workspace: WorkspaceSummary, client: DaemonClient) -> some View {
        switch node.target {
        case .task(let id)?:
            if let row = boardStore(for: workspace, client: client, host: host).board.rows.first(where: { $0.id == id }) {
                let agents = boardAgents(host: host, client: client)
                TaskRowMenu(
                    row: row, live: agents.live(for: row), orchestrator: agents.orchestrator(for: row),
                    onGoTo: { go(to: $0) }, ask: askOrchestrator(host: host, workspace: workspace))
            }
        case .terminal(let wt, let id)?:
            if let found = worktree(host: host, id: wt), let pane = found.terminals.first(where: { $0.id == id }) {
                Button("Open") { goInTree(.terminal(worktree: wt, terminal: id), host: host, workspace: workspace) }
                if client.daemonBuild?.can(.terminalNames) == true {
                    Button("Rename…") { Task { await run(.rename, on: pane, in: found) } }
                }
                Button("Close") { Task { await run(.close, on: pane, in: found) } }
            }
        default:
            if node.id == "group:main", let checkout = ProjectTerminals.checkout(for: workspace, host: host, in: store.fleet),
                store.refusal(for: host) == nil
            {
                Button("New Terminal") { Task { await openShell(besideOrchestratorIn: checkout, workspace: workspace.id) } }
            } else if let id = node.worktreeID, let found = worktree(host: host, id: id) {
                WorktreeMenuItems(items: worktreeMenu(for: found), perform: { perform($0, on: found) })
            }
        }
    }

    /// What `workspace`'s tree is built from: its board's cards, its plan
    /// and pages, its worktrees and the project's checkout, and its own
    /// Needs You, whose count is the title bar's.
    func oneTreeInput(
        host: String, workspace: WorkspaceSummary, client: DaemonClient, filter: OneTreeFilter
    ) -> OneTreeInput {
        let board = boardStore(for: workspace, client: client, host: host)
        let worktrees = boardWorktrees(host: host, workspace: workspace, client: client, board: board.board)
        var seen = Set<String>()
        let all = (Array(worktrees.byTask.values) + worktrees.shown + worktrees.hidden).filter {
            seen.insert($0.id).inserted && !$0.isMainCheckout
        }
        let checkout = ProjectTerminals.checkout(for: workspace, host: host, in: store.fleet)
        let needs = planNeedsYou(host: host, workspace: workspace)
        let plan = board.plan.plan
        return OneTreeInput(
            tasks: board.board.rows.map(OneTreeTask.init(row:)),
            plan: plan,
            worktrees: all.map { Self.treeWorktree($0, fleet: store.fleet) },
            mainCheckout: checkout.map { checkout in
                OneTreeWorktree(
                    id: checkout.id, name: checkout.task, path: checkout.path, branch: checkout.branch,
                    isMainCheckout: true,
                    terminals: ProjectTerminals.terminals(in: checkout, fleet: store.fleet).map(Self.treeTerminal))
            },
            pages: board.plan.listedPages.map(OneTreePage.init),
            unreadable: board.board.unreadable.map {
                OneTreeUnreadable(id: $0.id, key: $0.key, title: $0.title, status: $0.status)
            },
            asks: OneTreeAsks(items: needs.items, plan: plan),
            filter: filter,
            needsYouCount: needsYouCount(
                host: host, workspace: workspace, client: client, columnCount: board.board.waitingOnYou))
    }

    /// A worktree as the tree reads it: its own panes, not an orchestrator
    /// seated in it, and the cards it's working.
    nonisolated static func treeWorktree(_ worktree: Worktree, fleet: Fleet) -> OneTreeWorktree {
        let terminals = BoardWorktrees.terminals(of: worktree, in: fleet)
        return OneTreeWorktree(
            id: worktree.id, name: worktree.task, path: worktree.path, branch: worktree.branch,
            isMainCheckout: worktree.isMainCheckout, isHidden: worktree.isHidden,
            terminals: terminals.map(treeTerminal),
            taskIDs: worktree.openTaskIDs + worktree.terminals.compactMap(\.taskId))
    }

    nonisolated static func treeTerminal(_ terminal: Terminal) -> OneTreeTerminal {
        OneTreeTerminal(
            id: terminal.id, title: terminal.label,
            isAgent: isAgent(terminal), isOrchestrator: terminal.isOrchestrator)
    }

    /// Whether the tree draws `terminal` as an agent's, and so the jump
    /// bar's menu does, in one glyph (ov-328, `OneTreeGlyph.terminal`).
    nonisolated static func isAgent(_ terminal: Terminal) -> Bool {
        terminal.isAgentPane || AgentActivity.parse(terminal.activity).isAgent
    }

    /// The node `selection` is, in `workspace`'s tree: the plan for the
    /// workspace alone, a task, a plan page, a terminal or a worktree open.
    nonisolated static func treeTarget(_ selection: Selection?, workspace: String) -> OneTreeTarget? {
        switch selection {
        case .workspace(_, workspace, nil)?:
            return .plan
        case .workspace(_, workspace, .task(let id)?)?:
            return .task(id)
        case .workspace(_, workspace, .plan(let page)?)?:
            switch page {
            case .theme(let id): return .theme(id)
            case .lane(let id): return .lane(id)
            case .page(let slot): return .page(slot)
            case .needsYou: return .needsYou
            }
        case .workspace(_, workspace, .worktree(let id, let terminal)?)?, .looseWorktree(_, let id, let terminal)?:
            return terminal.map { .terminal(worktree: id, terminal: $0) } ?? .worktree(id)
        default:
            return nil
        }
    }

    /// The node `place` is, with a worktree opened whole read as the pane
    /// selected in it: the terminal row, not the lane's worktree.
    func treeTarget(_ place: Selection?, workspace: String) -> OneTreeTarget? {
        let target = Self.treeTarget(place, workspace: workspace)
        if case .worktree(let id)? = target, let pane = selectedPane, pane.worktree == id {
            return .terminal(worktree: id, terminal: pane.terminal)
        }
        return target
    }

    /// Where `target` is, as a selection in `workspace`. Nil for the
    /// orchestrator, which is focused rather than gone to.
    func treeSelection(_ target: OneTreeTarget, host: String, workspace: String) -> Selection? {
        switch target {
        case .orchestrator: return nil
        case .plan: return .workspace(host: host, workspace: workspace, focus: nil)
        case .needsYou: return .workspace(host: host, workspace: workspace, focus: .plan(.needsYou))
        case .theme(let id): return .workspace(host: host, workspace: workspace, focus: .plan(.theme(id)))
        case .lane(let id): return .workspace(host: host, workspace: workspace, focus: .plan(.lane(id)))
        case .page(let slot): return .workspace(host: host, workspace: workspace, focus: .plan(.page(slot)))
        case .task(let id): return .workspace(host: host, workspace: workspace, focus: .task(id))
        case .worktree(let id):
            return worktree(host: host, id: id).map { Self.opening($0, terminal: nil, in: store.fleet) }
        case .terminal(let id, let terminal):
            return worktree(host: host, id: id).map { Self.opening($0, terminal: terminal, in: store.fleet) }
        }
    }

    /// A node chosen: each by the route its kind already has, so a task
    /// chosen in the tree opens as one chosen anywhere else does.
    func goInTree(_ target: OneTreeTarget, host: String, workspace: WorkspaceSummary) {
        switch target {
        case .orchestrator:
            focusWorkspaceColumn(.focusConversation)
        case .plan:
            let next = Selection.workspace(host: host, workspace: workspace.id, focus: nil)
            guard next != selection else { return }
            trail = nil
            focusColumn = false
            selection = next
        case .needsYou: openPlan(.needsYou, host: host, workspace: workspace.id)
        case .theme(let id): openPlan(.theme(id), host: host, workspace: workspace.id)
        case .lane(let id): openPlan(.lane(id), host: host, workspace: workspace.id)
        case .page(let slot): openPlan(.page(slot), host: host, workspace: workspace.id)
        case .task(let id): chooseTask(id, host: host, workspace: workspace.id, glance: true)
        case .worktree(let id):
            if let found = worktree(host: host, id: id) { glance(at: found) }
        case .terminal(let id, let terminal):
            guard let found = worktree(host: host, id: id) else { return }
            if found.isMainCheckout, let pane = found.terminals.first(where: { $0.id == terminal }) {
                openProjectTerminal(pane, in: found, host: host, workspace: workspace)
            } else {
                glance(at: found, terminal: terminal)
            }
        }
    }

    /// The tree as the jump bar and ⌘↑ read it: every card, so a finished
    /// one opened from History still has its path.
    func pathTree(host: String, workspace: WorkspaceSummary) -> OneTree? {
        guard !showsBoardList, let client = store.clients[host] else { return nil }
        // From the board's cache: the jump bar and ⌘↑'s menu state each ask
        // on every draw, and build only when what the tree reads changed.
        return boardStore(for: workspace, client: client, host: host).plan.oneTree(
            oneTreeInput(host: host, workspace: workspace, client: client, filter: .all))
    }

    /// The jump bar's crumbs through the tree for `place`: the workspace,
    /// then Theme › Task › Lane › Terminal, each a way there. Nil where the
    /// tree has no path, which leaves the jump bar as it was.
    func treeCrumbs(for place: Selection, host: String, workspace: WorkspaceSummary?, name: String)
        -> [WorkspaceNavigation.Crumb]?
    {
        guard let workspace, let target = treeTarget(place, workspace: workspace.id),
            let tree = pathTree(host: host, workspace: workspace)
        else { return nil }
        let path = tree.crumbs(to: target, hint: treeHint)
        guard path.count > 1 || (path.count == 1 && target != .plan) else { return nil }
        let top = WorkspaceNavigation.Crumb(title: name, target: .workspace(host: host, workspace: workspace.id, focus: nil))
        return Self.crumbs(top: top, through: path) { treeSelection($0, host: host, workspace: workspace.id) }
    }

    /// The crumbs after the workspace's, one for each node on `path`, each
    /// a way there but the last, and each with the glyph its row has in the
    /// sidebar (ov-328). Pure, so a test reads them as the bar does.
    nonisolated static func crumbs(
        top: WorkspaceNavigation.Crumb, through path: [(title: String, target: OneTreeTarget?, glyph: String)],
        resolve: (OneTreeTarget) -> Selection?
    ) -> [WorkspaceNavigation.Crumb] {
        [top] + path.enumerated().map { index, crumb in
            let last = index == path.count - 1
            return WorkspaceNavigation.Crumb(
                title: crumb.title, target: last ? nil : crumb.target.flatMap(resolve), glyph: crumb.glyph)
        }
    }

    /// ⌘↑'s destination: the node over the selection's in the tree.
    var treeParent: Selection? {
        guard let scene = selection.flatMap(workspaceScene), let summary = scene.summary,
            let target = treeTarget(selection, workspace: summary.id),
            let parent = pathTree(host: scene.host, workspace: summary)?.parent(of: target, hint: treeHint)
        else { return nil }
        return treeSelection(parent, host: scene.host, workspace: summary.id)
    }

    /// ⌘↑: up the tree to the node over the selection's.
    func goUpTree() {
        // ⌘↑ in text being typed is the text's "to the beginning", which
        // the menu's key took first (review L1): handed back to it.
        if let text = NSApp.keyWindow?.firstResponder as? NSTextView {
            text.moveToBeginningOfDocument(nil)
            return
        }
        guard let next = treeParent, next != selection else { return }
        // The parent's own row, so a lane under two cards goes back up the
        // card it was reached through.
        if let hint = treeHint, let cut = hint.range(of: "/", options: .backwards) {
            treeHint = String(hint[..<cut.lowerBound])
        }
        trail = nil
        focusColumn = false
        selection = next
    }
}

/// The workspace's Needs You on the canvas, answered in place: where the one
/// tree's Needs You row opens (ov-321).
struct PlanNeedsYouPage: View {
    let needsYou: PlanNeedsYou
    /// The board: its plan's themes that ask for the owner, which the
    /// runner's list doesn't carry (review M3), and its Unread, which the
    /// tree has no place for (review M2: Unread lives here now).
    @ObservedObject var board: TaskBoardStore
    var onOpenTheme: (String) -> Void = { _ in }
    var onOpenTask: (String) -> Void = { _ in }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.group) {
                Text(OneTreeWords.needsYou)
                    .font(.system(size: WorkspaceStyle.PaneText.title, weight: .semibold))
                ForEach(board.plan.plan.shownThemes.filter { !$0.ownerAsk.isEmpty }) { theme in
                    Button { onOpenTheme(theme.id) } label: {
                        VStack(alignment: .leading, spacing: Spacing.tight) {
                            Text(theme.name).font(.system(size: WorkspaceStyle.PaneText.secondary)).foregroundStyle(.secondary)
                            PlanAsk(text: theme.ownerAsk)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .identified("plan-needs-you-theme-\(theme.name)")
                }
                if needsYou.items.isEmpty && !board.plan.plan.shownThemes.contains(where: { !$0.ownerAsk.isEmpty }) {
                    Text("Nothing in this workspace needs you.")
                        .foregroundStyle(.secondary)
                        .identified("plan-needs-you-empty")
                }
                ForEach(needsYou.items) { item in
                    NeedsYouItemRow(
                        item: item, canAct: needsYou.canAct(item), onOpen: { needsYou.onOpen(item) },
                        onAnswerAsk: { await needsYou.onAnswerAsk(item, $0) },
                        onDecide: { await needsYou.onDecide(item, $0) })
                    .changeWashed(item.key)
                }
                if board.hasRead {
                    // What changed on cards since they were last read: Unread
                    // lives here now the tree has taken the list's place.
                    BoardSummaryStrip(store: board, onChooseLine: { onOpenTask(BoardSummaryStrip.task(ofLine: $0)) })
                        .padding(.top, Spacing.group)
                        // One key column for the board, as the navigator sets.
                        .environment(\.taskKeyWidth, TaskKeyColumn.width(for: board.board.rows.map(\.key)))
                        .identified("plan-needs-you-unread")
                }
            }
            .listChanges(needsYou.items.map { ListChangeRow(id: $0.key, signature: "\($0.kind)\u{1}\($0.question)") })
            .padding(.horizontal, ColumnGrid.a + ColumnGrid.step)
            .padding(.vertical, 2 * ColumnGrid.rhythm)
            .frame(maxWidth: .infinity, alignment: .topLeading)
        }
        .scrollBounceBehavior(.basedOnSize)
        .contentCard()
        .identified("plan-needs-you-page")
    }
}
