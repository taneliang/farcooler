import AgentKit
import SwiftUI

/// Where the Plan view's pages open, and what they need from the window
/// (ov-273).
extension ContentView {
    /// A theme's or lane's page, in the main area where a task opens. The
    /// navigator keeps the keyboard, as it does for the History page.
    func openPlan(_ page: PlanPage, host: String, workspace: String) {
        let next = Selection.workspace(host: host, workspace: workspace, focus: .plan(page))
        guard next != selection else { return }
        trail = nil
        focusColumn = false
        selection = next
    }

    /// Whether `scene` is drawn as a canvas beside the chat (ov-298): a
    /// workspace's, with a board on a runner this window reaches. The one
    /// rule for what's drawn and what's seen and keyed.
    func splits(_ scene: WorkspaceScene) -> Bool {
        scene.board != nil && scene.summary != nil && store.clients[scene.host] != nil
    }

    /// The canvas's tiers as this window keeps them: the navigator's
    /// width, the chat's columns, the terminal font's cell.
    var canvasSizing: WorkspaceColumns.Canvas {
        WorkspaceColumns.Canvas(
            navigator: CGFloat(navigatorWidth), chatColumns: WorkspaceColumns.chatColumns(chatColumns),
            cell: TerminalMetrics.cell(preferences.terminalFont()).width)
    }

    /// The canvas's home, the strip and the peek for `scene` (ov-298):
    /// nil where the scene has no board to plan.
    func planViews(_ scene: WorkspaceScene) -> (home: AnyView, strip: AnyView, peek: AnyView)? {
        guard let summary = scene.summary, let client = store.clients[scene.host] else { return nil }
        let host = scene.host
        let board = boardStore(for: summary, client: client, host: host)
        let needs = planNeedsYou(host: host, workspace: summary)
        let open = { (page: PlanPage) in openPlan(page, host: host, workspace: summary.id) }
        let rulings = planRulingActions(host: host, workspace: summary, plan: board.plan)
        return (
            AnyView(PlanHome(board: board, needsYou: needs, onOpen: open).environment(\.planRulingActions, rulings)),
            AnyView(
                PlanStrip(
                    plan: board.plan,
                    needsYou: needs.items.count + board.plan.plan.shownThemes.filter { !$0.ownerAsk.isEmpty }.count
                ) { planPeeking = true }),
            AnyView(
                PlanPeek(board: board, needsYou: needs, onOpen: open) { planPeeking = false }
                    .environment(\.planRulingActions, rulings))
        )
    }

    /// `workspace`'s own Needs You, answered in place as the Needs You page
    /// answers (the toolbar's tray stays every workspace's).
    func planNeedsYou(host: String, workspace: WorkspaceSummary) -> PlanNeedsYou {
        PlanNeedsYou(
            items: store.needsYou.filter { WorkspaceCounts.count(for: workspace, host: host, in: [$0]) > 0 },
            canAct: { item in
                store.refusal(for: item.runner) == nil
                    && TaskBoardWrites.offered(by: store.clients[item.runner]?.daemonBuild)
            },
            onOpen: { open($0) },
            onAnswerAsk: { item, option in
                guard let client = store.clients[item.runner], let terminal = item.terminal, let ask = item.askID
                else { return .failed }
                return await client.answerAsk(terminal: terminal.id, request: ask, option: option)
            },
            onDecide: { item, body in
                guard let client = store.clients[item.runner], let task = item.task, let repository = item.repositoryID
                else { return false }
                return await client.answerDecision(key: task.key, body: body, repository: repository) == nil
            })
    }

    /// ⌥⌘P: the plan. Beside the chat, the canvas goes back to it; with the
    /// canvas folded away, it's peeked over the chat, or put away.
    func showPlan() {
        guard let scene = selection.flatMap(workspaceScene), scene.hasConversation, let board = scene.board else { return }
        if canvasSizing.hasCanvas(detailWidth ?? 0) {
            planPeeking = false
            if selection?.focus != nil {
                trail = nil
                selection = .workspace(host: scene.host, workspace: board, focus: nil)
            }
        } else {
            planPeeking.toggle()
        }
    }

    /// The page open in `workspace`'s main area, if it's a plan page.
    func planPage(host: String, workspace: String) -> PlanPage? {
        if case .workspace(host, workspace, .plan(let page)?)? = selection { return page }
        return nil
    }

    /// What a plan page reads from the board: its rows, and where a task,
    /// a lane or a theme opens.
    func planContext(_ board: TaskBoardStore, host: String, workspace: String) -> PlanPageContext {
        let worktrees = store.fleet.worktrees.filter { ($0.host ?? "") == host && $0.workspace == workspace }
        return PlanPageContext(
            rows: Dictionary(board.board.rows.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a }),
            onTask: { row in openTask(row.id, host: host, workspace: workspace) },
            onOpen: { page in openPlan(page, host: host, workspace: workspace) },
            worktrees: worktrees,
            onDestination: { destination in
                // A question still waiting opens in Needs You, where it's
                // answered (design 3.4, the owner's ruling on Q3).
                if let item = Self.askItem(destination, in: store.needsYou) {
                    lastAttention = item.key
                    selection = .needsYou
                    return
                }
                if let next = Self.pageSelection(destination, host: host, workspace: workspace, worktrees: worktrees) {
                    openFromPage(next, host: host, workspace: workspace)
                }
            })
    }

    /// The Needs You item an `ask` reference opens: its task's question,
    /// while it's still listed there. Nil for any other reference, or a
    /// question no longer waiting, which opens its task instead.
    static func askItem(_ destination: PageDestination, in items: [NeedsYouItem]) -> NeedsYouItem? {
        guard case .ask(let id) = destination else { return nil }
        let mine = items.filter { $0.task?.id == id }
        return mine.first { $0.kind == .decision } ?? mine.first { $0.kind == .ask }
    }

    /// Where a reference on an orchestrator's page goes (ov-284), past a
    /// waiting question (`askItem`): a card, or a question no longer
    /// waiting, to the task, a lane, a theme or a page to its page, a
    /// worktree whole, a terminal to its pane. A link to the web never
    /// reaches here: `PageView` hands it to the system.
    static func pageSelection(
        _ destination: PageDestination, host: String, workspace: String, worktrees: [Worktree]
    ) -> Selection? {
        switch destination {
        case .task(let id), .ask(let id):
            return .workspace(host: host, workspace: workspace, focus: .task(id))
        case .lane(let id):
            return .workspace(host: host, workspace: workspace, focus: .plan(.lane(id)))
        case .theme(let id):
            return .workspace(host: host, workspace: workspace, focus: .plan(.theme(id)))
        case .page(let slot):
            return .workspace(host: host, workspace: workspace, focus: .plan(.page(slot)))
        case .worktree(let id):
            return .workspace(host: host, workspace: workspace, focus: .worktree(id, terminal: nil))
        case .terminal(let id, let name):
            let pane = worktrees.first { $0.id == id }?.terminals.first { $0.title == name }?.id
            return .workspace(host: host, workspace: workspace, focus: .worktree(id, terminal: pane))
        case .url:
            return nil
        }
    }

    /// Open what a page named: a task as the palette opens one, the rest
    /// in the main area with the navigator keeping the keyboard.
    private func openFromPage(_ next: Selection, host: String, workspace: String) {
        if case .workspace(_, _, .task(let id)?) = next {
            openTask(id, host: host, workspace: workspace)
            return
        }
        guard next != selection else { return }
        trail = nil
        focusColumn = false
        selection = next
    }
}
