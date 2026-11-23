import AgentKit
import SwiftUI

/// Where the Plan view's pages open, and what they need from the window
/// (ov-273).
extension ContentView {
    /// A theme's or ticket's page, in the main area where a task opens. The
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
        let actions = planColumnActions(host: host, workspace: summary)
        return (
            AnyView(
                PlanHome(board: board, needsYou: needs, onOpen: open, actions: actions)
                    .environment(\.planRulingActions, rulings)),
            AnyView(
                PlanStripRow(
                    plan: board.plan,
                    needsYou: needs.items.count + board.plan.plan.shownThemes.filter { !$0.ownerAsk.isEmpty }.count,
                    statuses: board.board.statuses
                ) { planPeeking = true }),
            AnyView(
                PlanPeek(board: board, needsYou: needs, onOpen: open, actions: actions) { planPeeking = false }
                    .environment(\.planRulingActions, rulings))
        )
    }

    /// What the plan column's rows do (ov-867): a Needs You row is quoted into
    /// the orchestrator's input, unsent, as Ask the Orchestrator does; an
    /// agent opens its chat, else its ticket's page; a ticket opens as a row
    /// clicked in the navigator does; a footer count opens the board's list.
    func planColumnActions(host: String, workspace: WorkspaceSummary) -> PlanColumnActions {
        let live = WorkspaceScreen.workspace(
            workspace.id, host: host, in: store.fleet,
            repositories: store.clients[host]?.repositories.map(\.id) ?? []) ?? workspace
        let seat = surfaces(host: host, workspace: live).asksOrchestrator
            ? WorkspaceScreen.orchestrator(of: live, host: host, in: store.fleet) : nil
        let worktrees = store.fleet.worktrees.filter { ($0.host ?? "") == host }
        return PlanColumnActions(
            quote: { text in
                guard let seat else {
                    errorBanner = "Start the orchestrator to talk to it about this."
                    return
                }
                let client = store.client(for: seat.worktree)
                Task { @MainActor in
                    let pasted = await client?.draftPrompt(terminal: seat.terminal.short, text: text) ?? .declined
                    // The person finishes it in the pane, so the pane takes the keyboard.
                    focus(PaneRef(host: host, worktree: seat.worktree.id, terminal: seat.terminal.id))
                    if pasted == .declined {
                        (client?.copyToClipboard ?? AskOrchestrator.copyToPasteboard)(
                            text.trimmingCharacters(in: .whitespaces))
                        errorBanner = "Copied it. Paste it into the orchestrator."
                    }
                }
            },
            openAgent: { agent in
                if let id = agent.terminal, let ws = worktrees.first(where: { $0.terminals.contains { $0.id == id } }),
                    let terminal = ws.terminals.first(where: { $0.id == id })
                {
                    go(to: BoardPane(terminal: terminal, worktree: ws))
                } else if let ticket = agent.ticket {
                    openPlan(.ticket(ticket), host: host, workspace: workspace.id)
                } else {
                    errorBanner = "\(agent.name) has no chat to open."
                }
            },
            openTicket: { id in openTask(id, host: host, workspace: workspace.id) },
            // Under an agent's card, the breadcrumb names the agent (ov-873).
            openTicketUnder: { agent, id in
                openTask(id, host: host, workspace: workspace.id)
                var target: Selection?
                if let terminal = agent.terminal, let ws = worktrees.first(where: { $0.terminals.contains { $0.id == terminal } }) {
                    target = Self.opening(ws, terminal: terminal, in: store.fleet)
                } else if let ticket = agent.ticket {
                    target = .workspace(host: host, workspace: workspace.id, focus: .plan(.ticket(ticket)))
                }
                jumpBar.planOrigin = PlanStack.Origin(task: id, agent: agent.name, doing: agent.doing, target: target)
            },
            openList: { status in
                trail = nil
                selection = .workspace(host: host, workspace: workspace.id, focus: .history(status))
            },
            openRun: { key in openPlan(.engine(.ticket(key)), host: host, workspace: workspace.id) })
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
    /// a ticket or a theme opens.
    func planContext(_ board: TaskBoardStore, host: String, workspace: String) -> PlanPageContext {
        PlanPageContext(
            rows: Dictionary(board.board.rows.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a }),
            onTask: { row in openTask(row.id, host: host, workspace: workspace) },
            onOpen: { page in openPlan(page, host: host, workspace: workspace) })
    }
}
