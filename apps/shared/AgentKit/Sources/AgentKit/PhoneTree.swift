import Foundation

// The phone's side of the One tree and the plan strip (ov-300): what the
// tree is built from on a phone, the root it shows, what a row does when it's
// tapped, where each target goes on the phone's stack, and the orchestrator's
// state and line for the strip. The rules of the tree itself are `OneTree`'s,
// the Mac's sidebar's; this only maps the phone's own fleet types onto them
// and push navigation onto the outline.
//
// iPhone: Themes › Theme › Task › Lane › Terminal, one level a screen. A row
// with children pushes its level; a level opens on its node's own page; a
// leaf opens what it points at. See .claude/agent/reports/phones-tree/design.md.

enum PhoneTree {
    // MARK: What the tree is built from

    /// `summary`'s tree input, from what a phone's connection holds: its
    /// board, its plan and pages, the runner's worktrees, and its own
    /// Needs You (for the dots; `needsYouCount` is the strip's number).
    static func input(
        summary: WorkspaceSummary, board: TaskBoardModel?, plan: PlanModel, worktrees: [Worktree],
        pages: [BoardPage], items: [NeedsYouItem], filter: OneTreeFilter, needsYouCount: Int
    ) -> OneTreeInput {
        let rows = board?.rows ?? []
        let cards = Set(rows.map(\.id)).union(plan.cards.map(\.task))
        let repository = summary.repository ?? summary.id
        let checkout = worktrees.first { $0.isPrimaryCheckout && $0.repository == repository }
        // The workspace's own worktrees, and any elsewhere holding one of its
        // cards, as the Mac's tree reads `taskWorktrees` beside the loose ones.
        let kept = worktrees.filter { worktree in
            guard !worktree.isPrimaryCheckout else { return false }
            let owned = summary.isImplicit ? worktree.repository == summary.id : worktree.workspace == summary.id
            return owned || taskIDs(of: worktree).contains(where: cards.contains)
        }
        return OneTreeInput(
            tasks: rows.map(OneTreeTask.init(row:)),
            plan: plan,
            worktrees: kept.map(treeWorktree),
            mainCheckout: checkout.map { checkout in
                OneTreeWorktree(
                    id: checkout.id, name: checkout.task, path: checkout.worktree ?? "", branch: checkout.branch,
                    isMainCheckout: true,
                    // The project's own: not an orchestrator, not a task's
                    // agent, not a changes pane (the Mac's `ProjectTerminals`).
                    terminals: checkout.terminals.filter { !$0.isOrchestrator && $0.taskId == nil && !$0.isChangesPane }
                        .map(treeTerminal))
            },
            pages: pages.map(OneTreePage.init),
            unreadable: (board?.unreadable ?? []).map {
                OneTreeUnreadable(id: $0.id, key: $0.key, title: $0.title, status: $0.status)
            },
            asks: OneTreeAsks(items: items.filter { mine(summary, $0) }, plan: plan),
            filter: filter,
            needsYouCount: needsYouCount)
    }

    /// The workspace's Needs You count: the Mac's title bar's and tree's one
    /// number (`WorkspaceNeedsYou.count`), from one runner's list.
    static func needsYouCount(
        summary: WorkspaceSummary, board: TaskBoardModel?, plan: PlanModel, items: [NeedsYouItem],
        listRead: Bool, listServed: Bool
    ) -> Int {
        WorkspaceNeedsYou.count(
            items: items.filter { mine(summary, $0) }.count, columnCount: board?.waitingOnYou ?? 0,
            listRead: listRead, listServed: listServed,
            themeAsks: plan.shownThemes.filter { !$0.ownerAsk.isEmpty }.count)
    }

    /// Whether `item` is `summary`'s: an implicit board's are its
    /// repository's that name no workspace (`RunnerBoards.decisions`).
    static func mine(_ summary: WorkspaceSummary, _ item: NeedsYouItem) -> Bool {
        summary.isImplicit
            ? item.workspaceID == nil && item.repositoryID == summary.id : item.workspaceID == summary.id
    }

    static func taskIDs(of worktree: Worktree) -> [String] {
        worktree.openTaskIDs + worktree.terminals.compactMap(\.taskId)
    }

    static func treeWorktree(_ worktree: Worktree) -> OneTreeWorktree {
        OneTreeWorktree(
            id: worktree.id, name: worktree.task, path: worktree.worktree ?? "", branch: worktree.branch,
            isMainCheckout: worktree.isPrimaryCheckout, isHidden: worktree.isHidden,
            terminals: worktree.terminals.filter { !$0.isChangesPane }.map(treeTerminal),
            taskIDs: taskIDs(of: worktree))
    }

    static func treeTerminal(_ terminal: Terminal) -> OneTreeTerminal {
        OneTreeTerminal(
            id: terminal.id, title: terminal.label, isAgent: terminal.isAgentPane || terminal.agent.isAgent,
            isOrchestrator: terminal.isOrchestrator)
    }

    // MARK: The root

    /// The root screen's two sections: the work (the themes, No Theme, and
    /// cards this build can't read), then the main checkout and the loose
    /// worktrees. The pinned places are the phone's already: Needs You is
    /// the app's root and the strip's count, and Plan is the sheet.
    static func root(_ tree: OneTree) -> (work: [OneTreeNode], below: [OneTreeNode]) {
        (tree.tree, tree.below)
    }

    /// The node with `id`, anywhere in `tree`, or nil once it's gone.
    static func node(_ id: String, in tree: OneTree) -> OneTreeNode? {
        tree.allNodes.first { $0.id == id }
    }

    // MARK: Tapping

    /// What a tapped row does.
    enum Tap: Equatable {
        /// Push the level that lists this node's children.
        case push(String)
        /// Open what it points at.
        case open(OneTreeTarget)
        /// Nothing: a subagent, which has no pane, or a row with nowhere to go.
        case none
    }

    /// A row with children pushes its level; a leaf opens its target. A
    /// subagent opens nothing: it runs inside the orchestrator, and its
    /// caption says so, so it's never a chevron to a dead end.
    static func tap(_ node: OneTreeNode) -> Tap {
        if node.hasChildren { return .push(node.id) }
        // A subagent's target is the orchestrator, which goes nowhere here.
        guard let target = node.target else { return .none }
        switch target {
        case .orchestrator, .needsYou, .plan: return .none
        default: return .open(target)
        }
    }

    /// The first row of a level: the node's own page, so the thing itself
    /// is one tap from its level. Nil for a group with no page of its own.
    static func ownRow(_ node: OneTreeNode) -> String? {
        switch node.target {
        case .theme?: "Theme Page"
        case .task?: "Task Details"
        case .lane?: "Lane Page"
        case .worktree?: "Open Worktree"
        default: nil
        }
    }

    /// Where `target` goes on the phone's stack, in `place`'s workspace.
    /// Nil for the places the phone has elsewhere: the orchestrator is a
    /// segment, Needs You the root, Plan the sheet.
    static func route(_ target: OneTreeTarget, in place: PhoneWorkspace) -> PhoneRoute? {
        switch target {
        case .orchestrator, .needsYou, .plan: nil
        case .theme(let id): .plan(place, page: .theme(id))
        case .task(let id): .task(place, task: id)
        case .lane(let id): .plan(place, page: .lane(id))
        case .page(let slot): .plan(place, page: .page(slot))
        case .worktree(let id): .worktree(runner: place.runner, worktree: id, landing: .resume)
        case .terminal(let worktree, let terminal):
            .worktree(runner: place.runner, worktree: worktree, landing: .terminal(terminal))
        }
    }

    // MARK: The orchestrator, for the strip

    /// The orchestrator's state, from its pane as the phone holds it.
    static func orchestrator(_ terminal: Terminal?) -> PlanStripOrchestrator {
        guard let terminal else { return .none }
        switch StateKind.parse(terminal.state) {
        case .starting: return .starting
        case .lost, .exited, .error: return .stopped
        case .running, .unknown: break
        }
        if terminal.turnDidFail { return .failed }
        switch terminal.agent {
        case .blocked: return .needsYou
        case .working: return .working
        case .done: return .done
        case .idle, .none, .unknown: return .idle
        }
    }

    /// Its one line, for the peek: the question it's blocked on; working,
    /// what its hook says it's doing, else the last thing it said; failed,
    /// finished or idle, the last thing it said. A line that's only the
    /// runner's headline ("claude 4m") says nothing the state doesn't.
    static func line(_ terminal: Terminal?, state: PlanStripOrchestrator) -> String? {
        guard let terminal else { return nil }
        func text(_ s: String?) -> String? {
            guard let s = s?.trimmingCharacters(in: .whitespacesAndNewlines), !s.isEmpty else { return nil }
            return s
        }
        let headline = text(terminal.headline)
        let signal = text(terminal.signalLine).flatMap { $0 == headline ? nil : $0 }
        switch state {
        case .none, .starting, .stopped: return nil
        case .needsYou: return text(terminal.blockedQuestion) ?? signal
        case .working: return signal ?? text(terminal.lastSaid)
        case .failed, .done, .idle: return text(terminal.lastSaid)
        }
    }
}
