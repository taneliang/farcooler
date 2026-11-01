import Combine
import SwiftUI

// The Plan on an iPhone, as the Mac's plan column draws it (ov-874, after
// ov-867's C10f): the facts line, Needs You, Initiatives, the pipeline
// (Stuck, Building, Reviewing, Landing, each agent a soft card with its tickets
// indented under it) and the footer counts.
//
// Every rule is AgentKit's `PlanColumn`; this draws it in iOS's idiom, a
// native inset-grouped list. Nothing here acts on an agent: no Stop, Assign,
// cost or rules. Stopping, assigning and answering are said to the
// orchestrator, and a Needs You row leaves its words in the orchestrator's
// composer, unsent.

/// The Plan list for a workspace whose runner keeps a plan.
struct PhonePlanColumn<Top: View>: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    /// In the sheet over a conversation, which a quote closes onto.
    let inSheet: Bool
    @ViewBuilder let top: () -> Top

    @ObservedObject private var reads: PlanReads
    @ObservedObject private var pageReads: PageReads
    @Environment(\.phoneNavigator) private var navigator
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var composing = false
    @State private var fromBranch = false
    @State private var moveFailure: ActionFailure?
    /// The worktrees it last drew (ov-791): under a pushed screen it keeps
    /// them rather than building the tree on every board move.
    @State private var held = HeldTree()
    @State private var moves = 0
    /// Whether Past Decisions is open: here, so a re-read can't close it.
    @State private var pastRulingsOpen = false
    /// What the last quote left on the clipboard, when the runner couldn't paste.
    @State private var notice: String?

    init(
        connection: Connection, summary: WorkspaceSummary, place: PhoneWorkspace, inSheet: Bool,
        @ViewBuilder top: @escaping () -> Top
    ) {
        self.connection = connection
        self.summary = summary
        self.place = place
        self.inSheet = inSheet
        self.top = top
        reads = connection.plans
        pageReads = connection.pages
    }

    var body: some View {
        List {
            top()
            content
        }
        .listStyle(.insetGrouped)
        .listSectionSpacing(.compact)
        .environment(\.defaultMinListRowHeight, PlanRowHeight.minimum)
        .task {
            await connection.readTree(summary)
            if connection.keepsPages { await connection.readPages(summary) }
        }
        .onReceive(navigator?.objectWillChange.eraseToAnyPublisher() ?? Empty<Void, Never>().eraseToAnyPublisher()) { _ in moves &+= 1 }
        .refreshable {
            _ = await connection.readBoard(summary)
            await connection.readPlan(summary)
            if connection.keepsPages { await connection.readPages(summary) }
        }
        .actionFailureAlert($moveFailure)
        .newWorktreeSheets(composing: $composing, fromBranch: $fromBranch, connection: connection, summary: summary)
        .environment(\.taskKeyLinker, connection.taskKeyLinker(navigator))
    }

    private func read() async { await connection.readPlan(summary) }

    @ViewBuilder private var content: some View {
        switch reads.state(summary.id) {
        case .none, .loading:
            Section {
                ProgressView()
                    .frame(maxWidth: .infinity, alignment: .center)
                    .accessibilityLabel("Reading the plan")
                    .accessibilityIdentifier("plan-loading")
            }
        case .needsUpdate:
            Section { PlanNotice(title: PlanWords.needsUpdate, detail: nil).accessibilityIdentifier("plan-needs-update") }
        case .reconnecting:
            Section {
                HStack(spacing: 8) {
                    ProgressView()
                    Text(PlanWords.reconnecting).foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("plan-reconnecting")
                Button(PlanWords.tryAgain) { Task { await read() } }.accessibilityIdentifier("plan-retry")
            }
        case .unavailable:
            Section {
                PlanNotice(title: PlanWords.couldntRead, detail: nil).accessibilityIdentifier("plan-unavailable")
                Button(PlanWords.tryAgain) { Task { await read() } }.accessibilityIdentifier("plan-retry")
            }
        case .loaded(let plan):
            loaded(plan)
        }
    }

    // MARK: What it says

    /// The themes asking for the owner, counted with Needs You.
    private func asks(_ plan: PlanModel) -> [PlanTheme] { plan.shownThemes.filter { !$0.ownerAsk.isEmpty } }

    /// The runner's own Needs You list for this workspace, most urgent first.
    private var items: [NeedsYouItem] {
        connection.needsYou.filter { PhoneTree.mine(summary, $0) }.sorted { $0.rank < $1.rank }
    }

    @ViewBuilder private func loaded(_ plan: PlanModel) -> some View {
        let asks = asks(plan)
        let items = items
        if plan.isEmpty && items.isEmpty && asks.isEmpty {
            // Rulings alone plan nothing, but they are something to show.
            if !(connection.keepsRulings && !plan.rulings.isEmpty) {
                Section {
                    PlanNotice(title: PlanWords.nothingPlanned, detail: PlanWords.nothingPlannedDetail)
                        .accessibilityIdentifier("plan-empty")
                }
            }
        } else {
            let rows = connection.boards[summary.id]?.rows ?? []
            let column = PlanColumn(
                plan: plan, needsYou: items.count + asks.count,
                statuses: Dictionary(rows.map { ($0.id, $0.status) }, uniquingKeysWith: { first, _ in first }),
                titles: Dictionary(rows.map { ($0.id, $0.title) }, uniquingKeysWith: { first, _ in first }))
            PhonePlanFacts(facts: column.facts)
            needsYou(asks: asks, items: items)
            PhonePlanInitiatives(column: column) { navigator?.open(.plan(place, page: .theme($0))) }
            PhonePlanPipeline(column: column, actions: actions)
            PhonePlanFooter(counts: column.footer) {
                navigator?.open(.history(place, status: $0.kind == .done ? TaskStatus.done.rawValue : $0.kind.rawValue))
            }
        }
        rulings(plan)
        pages(plan)
        worktrees
    }

    /// Decided For You (ov-304), under the footer as on the web: the calls the
    /// orchestrator made for the owner, with Keep, Reverse and Discuss.
    @ViewBuilder private func rulings(_ plan: PlanModel) -> some View {
        PlanRulingsSection(
            plan: plan, keeps: connection.keepsRulings,
            actions: connection.rulingActions(
                summary, place: place,
                open: { [navigator, inSheet] route in
                    // Reverse and Discuss draft into the orchestrator's pane,
                    // under the sheet: closing it is the way there (ov-643).
                    if inSheet { navigator?.plan = nil } else { navigator?.open(route) }
                }),
            pastOpen: $pastRulingsOpen)
    }

    /// The orchestrator's pages, which the plan's list has always led to.
    @ViewBuilder private func pages(_ plan: PlanModel) -> some View {
        if connection.keepsPages {
            PlanPagesSection(
                state: pageReads.state(summary.id), plan: plan, read: read,
                onOpen: { navigator?.open(.plan(place, page: $0)) })
        }
    }

    /// The main checkout and the loose worktrees, and New Worktree: where a
    /// terminal that no agent is on is found.
    @ViewBuilder private var worktrees: some View {
        let below = belowTree
        if !below.isEmpty {
            Section {
                ForEach(below) { node in TreeRow(node: node, connection: connection, place: place, failure: $moveFailure) }
            }
        }
        TreeWorktreeActions(connection: connection, composing: $composing, fromBranch: $fromBranch)
    }

    private var belowTree: [OneTreeNode] {
        _ = moves
        let covered = sizeClass == .compact && navigator?.isOnTop(inSheet ? nil : .workspace(place)) == false
        if covered, let kept = held.root { return kept.below }
        let made = PhoneTree.root(connection.oneTree(summary, filter: .open, narrow: nil))
        held.root = made
        return made.below
    }

    private var actions: PhonePlanActions {
        PhonePlanActions(
            openAgent: { agent in
                // Its chat or terminal, else its ticket's page.
                if let terminal = agent.terminal,
                    let home = connection.fleet.worktrees.first(where: { $0.terminals.contains { $0.id == terminal } })
                {
                    navigator?.open(.worktree(runner: place.runner, worktree: home.id, landing: .terminal(terminal)))
                } else if let ticket = agent.ticket {
                    navigator?.open(.task(place, task: ticket))
                }
            },
            openTicket: { navigator?.open(.task(place, task: $0)) },
            openStage: { navigator?.open(.history(place, status: PlanStageRoute.status($0))) })
    }

    /// Needs You: one line each. A tap leaves it in the orchestrator's input.
    @ViewBuilder private func needsYou(asks: [PlanTheme], items: [NeedsYouItem]) -> some View {
        let count = asks.count + items.count
        if count > 0 {
            Section {
                ForEach(asks) { theme in
                    PhoneNeedsYouLine(
                        title: theme.name, detail: theme.ownerAsk, since: nil,
                        quote: { quote(PlanColumn.quote(key: nil, title: "\(theme.name): \(theme.ownerAsk)")) }
                    )
                    .accessibilityIdentifier("plan-needs-theme-\(theme.name)")
                }
                ForEach(items) { item in
                    let title = item.task?.title ?? item.question
                    PhoneNeedsYouLine(
                        title: title, detail: item.task == nil ? (item.distinctDetail ?? "") : item.question, since: item.since,
                        quote: { quote(PlanColumn.quote(key: item.task?.key, title: title)) }
                    )
                    .accessibilityIdentifier("plan-needs-\(item.key)")
                }
            } header: {
                PhonePlanHeading(title: "Needs You", count: "\(count)").accessibilityIdentifier("plan-needs-you")
            } footer: {
                if let notice { Text(notice).accessibilityIdentifier("plan-quote-notice") }
            }
        }
    }

    private func quote(_ text: String) {
        notice = nil
        Task { @MainActor in
            notice = await connection.quoteInOrchestrator(
                text, summary: summary, place: place, inSheet: inSheet, navigator: navigator)
        }
    }
}

extension Connection {
    /// Leave `text` in the orchestrator's composer, unsent, and go there
    /// (the sheet closes onto it). Returns what to say when it was copied
    /// instead, because the runner couldn't paste it.
    @MainActor
    func quoteInOrchestrator(
        _ text: String, summary: WorkspaceSummary, place: PhoneWorkspace, inSheet: Bool, navigator: PhoneNavigator?
    ) async -> String? {
        // With no orchestrator up it can only be copied, and says so.
        guard WorkspaceScreen.orchestratorIsUp(in: self, summary: summary),
            let seat = OrchestratorSegment.terminal(in: self, summary: summary)
        else {
            UIPasteboard.general.string = text.trimmingCharacters(in: .whitespaces)
            return "Copied to the clipboard. Start an orchestrator to send it."
        }
        let delivery = await AskAboutTask.deliver(
            text: text, paste: { await draftPrompt(terminal: seat.id, text: $0) }, copy: { UIPasteboard.general.string = $0 })
        if delivery == .copied { return "Copied to the clipboard. Paste it into the orchestrator." }
        if inSheet, navigator?.plan?.place == place {
            navigator?.plan = nil
        } else if !hasConversation(summary), let home = fleet.worktrees.first(where: { $0.terminals.contains { $0.id == seat.id } }) {
            navigator?.open(.worktree(runner: place.runner, worktree: home.id, landing: .terminal(seat.id)))
        }
        return nil
    }
}

/// What the plan's rows do on the phone.
struct PhonePlanActions {
    var openAgent: (PlanColumn.Agent) -> Void = { _ in }
    var openTicket: (String) -> Void = { _ in }
    /// The list of a stage's tickets nobody is on.
    var openStage: (PlanColumn.Stage) -> Void = { _ in }
}
