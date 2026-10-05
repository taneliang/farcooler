import SwiftUI

// A theme's page and a lane's page (ov-268 design 6.2, 6.3 and 6.5), pushed
// from the Plan view, in one column: the same sections as the Mac's pages.
// A card on either opens its task, as a card on the board does.

/// The page for `page`, or why there's none.
struct PlanPageScreen: View {
    @ObservedObject var connection: Connection
    @ObservedObject private var reads: PlanReads
    @ObservedObject private var pageReads: PageReads
    let place: PhoneWorkspace
    let page: PhonePlanPage
    @Environment(\.phoneNavigator) private var navigator

    init(connection: Connection, place: PhoneWorkspace, page: PhonePlanPage) {
        self.connection = connection
        reads = connection.plans
        pageReads = connection.pages
        self.place = place
        self.page = page
    }

    private var summary: WorkspaceSummary? { connection.workspace(place.workspace) }

    var body: some View {
        Group {
            switch reads.state(place.workspace) {
            case .none where summary == nil:
                // The workspace left while the page was open.
                ContentUnavailableView {
                    Label(PlanWords.couldntRead, systemImage: "exclamationmark.triangle")
                }
                .accessibilityIdentifier("plan-unavailable")
            case .none, .loading:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            case .needsUpdate:
                ContentUnavailableView(PlanWords.needsUpdate, systemImage: "map")
                    .accessibilityIdentifier("plan-needs-update")
            case .unavailable:
                ContentUnavailableView {
                    Label(PlanWords.couldntRead, systemImage: "exclamationmark.triangle")
                } actions: {
                    Button(PlanWords.tryAgain) { Task { await load() } }
                        .accessibilityIdentifier("plan-retry")
                }
                .accessibilityIdentifier("plan-unavailable")
            case .loaded(let plan):
                loaded(plan)
            }
        }
        // A task key in a page's text opens its task, as one in a note does.
        .environment(\.taskKeyLinker, connection.taskKeyLinker(navigator))
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        // Its cards' keys preview their tasks (ov-299).
        .environment(\.taskKeyLinker, connection.taskKeyLinker(navigator))
        .task(id: page) { await load() }
        .onAppear { reads.openPage = page }
        .onDisappear { if reads.openPage == page { reads.openPage = nil } }
    }

    private var title: String {
        guard let plan = reads.state(place.workspace)?.plan else { return page.word }
        switch page {
        case .theme(let id): return plan.themes.first { $0.id == id }?.name ?? page.word
        case .lane(let id): return plan.lanes.first { $0.id == id }?.name ?? page.word
        case .page(let slot): return pageReads.pages(place.workspace).first { $0.slot == slot }?.title ?? page.word
        }
    }

    private func load() async {
        guard let summary else { return }
        if reads.state(place.workspace)?.plan == nil { await connection.readPlan(summary) }
        // A page, and a theme's page that may draw pages inside it, read the
        // board's pages when nothing has yet.
        if connection.keepsPages, pageReads.state(place.workspace) == nil { await connection.readPages(summary) }
        await connection.readPlanRecord(page)
    }

    @ViewBuilder
    private func loaded(_ plan: PlanModel) -> some View {
        let context = PlanPageContext(
            rows: Dictionary(
                (connection.boards[place.workspace]?.rows ?? []).map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a }),
            onTask: { row in navigator?.open(.task(place, task: row.id)) },
            onOpen: { page in navigator?.open(.plan(place, page: page)) },
            pages: pageReads.pages(place.workspace),
            world: summary.map { connection.pageWorld($0) } ?? PageWorld(),
            onDestination: { destination in
                switch connection.route(destination, from: place) {
                case .push(let route)?: navigator?.open(route)
                case .needsYou?: navigator?.go([])
                case nil: break
                }
            })
        switch page {
        case .theme(let id):
            if let theme = plan.themes.first(where: { $0.id == id }) {
                PlanThemePage(theme: theme, plan: plan, record: reads.records[page], context: context)
            } else {
                ContentUnavailableView("Theme Not Found", systemImage: "map")
            }
        case .lane(let id):
            if let lane = plan.lanes.first(where: { $0.id == id }) {
                PlanLanePage(lane: lane, plan: plan, record: reads.records[page], context: context)
            } else {
                ContentUnavailableView("Lane Not Found", systemImage: "map")
            }
        case .page(let slot):
            if let found = context.pages.first(where: { $0.slot == slot }) {
                PlanOrchestratorPage(page: found, world: context.world, onDestination: context.onDestination)
            } else if pageReads.state(place.workspace) == .unavailable {
                ContentUnavailableView {
                    Label(PageWords.couldntRead, systemImage: "exclamationmark.triangle")
                } actions: {
                    Button(PlanWords.tryAgain) { Task { await load() } }
                        .accessibilityIdentifier("plan-pages-retry")
                }
            } else if pageReads.state(place.workspace) == nil {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                // Removed while it was open.
                ContentUnavailableView("Page Not Found", systemImage: "doc.richtext")
                    .accessibilityIdentifier("plan-page-gone")
            }
        }
    }
}

/// What a plan page needs from the board: its tasks, to draw a card as the
/// board draws it, and where a task, a lane or a theme opens.
struct PlanPageContext {
    let rows: [String: TaskRow]
    let onTask: (TaskRow) -> Void
    let onOpen: (PhonePlanPage) -> Void
    /// The board's orchestrator pages (ov-285): a page pushed, and those a
    /// theme's page draws inside it.
    var pages: [BoardPage] = []
    /// What a page's references are drawn from.
    var world = PageWorld()
    /// Where a page's reference goes.
    var onDestination: (PageDestination) -> Void = { _ in }
}

// MARK: - Theme

/// A theme's page: its outcome, where it stands and what changed, what's
/// next and what needs you, its lanes, and its cards.
struct PlanThemePage: View {
    let theme: PlanTheme
    let plan: PlanModel
    let record: PlanRecord?
    let context: PlanPageContext
    @State private var showingChange = false

    var body: some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: PaneMetrics.step) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(theme.name).font(.title3.weight(.semibold))
                        Spacer()
                        Text(theme.state.capitalized).font(.subheadline).foregroundStyle(.secondary)
                    }
                    if !theme.outcome.isEmpty {
                        Text(theme.outcome).font(.body).foregroundStyle(.secondary)
                    }
                    PlanProgressBar(counts: theme.counts)
                    Text(PlanWords.progress(theme.counts))
                        .font(.footnote.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 2)
                .accessibilityIdentifier("plan-theme-page")
            }
            story
            if !theme.next.isEmpty || !theme.ownerAsk.isEmpty {
                Section {
                    if !theme.next.isEmpty {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Next").font(.footnote).foregroundStyle(.secondary)
                            Text(theme.next)
                        }
                        .accessibilityElement(children: .combine)
                    }
                    if !theme.ownerAsk.isEmpty { PlanAsk(text: theme.ownerAsk).font(.body) }
                }
            }
            // The orchestrator's pages anchored here, after Needs You and
            // before Lanes (design 6.1).
            PlanAnchoredPages(
                pages: PageShelf.anchored(context.pages, to: theme.id, plan: plan), world: context.world,
                onOpen: context.onOpen, onDestination: context.onDestination)
            if PlanThemeSpend.hasSomething(theme) {
                Section("Spend") {
                    PlanThemeSpend(theme: theme, bodyFont: .body, secondaryFont: .footnote)
                }
            }
            let lanes = plan.lanes(in: theme)
            if !lanes.isEmpty {
                Section("Lanes") {
                    ForEach(lanes) { lane in
                        PlanPageLaneRow(lane: lane) { context.onOpen(.lane(lane.id)) }
                    }
                }
            }
            Section {
                PlanCardRows(refs: theme.cards, plan: plan, context: context)
            } header: {
                HStack {
                    Text("Cards")
                    Spacer()
                    Text(PlanWords.breakdown(theme.counts)).font(.footnote).textCase(nil)
                }
            }
        }
        .listStyle(.insetGrouped)
    }

    @ViewBuilder private var story: some View {
        Section {
            if theme.story.isEmpty {
                Text("The orchestrator hasn’t written where this stands yet.").foregroundStyle(.secondary)
            } else {
                Text(theme.story)
            }
            if showingChange, let before = record?.previousStory {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Before, \(PlanWords.ago(before.at, now: plan.nowMs))")
                        .font(.footnote.weight(.medium))
                        .foregroundStyle(.secondary)
                    Text(before.story).foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("plan-previous-story")
            }
            if record?.previousStory != nil {
                Button(showingChange ? "Hide Changes" : "What Changed") { showingChange.toggle() }
                    .accessibilityIdentifier("plan-what-changed")
            }
        } header: {
            HStack {
                Text("Where It Stands")
                Spacer()
                if theme.storyAt > 0 {
                    Text("Updated \(PlanWords.ago(theme.storyAt, now: plan.nowMs))")
                        .font(.footnote)
                        .textCase(nil)
                }
            }
        }
    }
}

/// A lane on a theme's page: its name, its state, and its cards' keys.
private struct PlanPageLaneRow: View {
    let lane: PlanLane
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.step) {
                Image(systemName: PlanGlyph.name(lane.state))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .frame(width: 22, alignment: .leading)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(lane.name).font(.subheadline.weight(.medium))
                    Text(PlanWords.status(lane)).font(.footnote).foregroundStyle(.secondary)
                    // Each key's title in what VoiceOver says (ov-299).
                    TaskKeyText(keysIn: lane.cards.map(\.key).joined(separator: " "))
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("plan-page-lane-\(lane.name)")
    }
}

/// Cards as the board draws them, from the board's rows; a card the board
/// hasn't read draws from the plan's read of it.
private struct PlanCardRows: View {
    let refs: [PlanCardRef]
    let plan: PlanModel
    let context: PlanPageContext

    var body: some View {
        ForEach(refs, id: \.self) { ref in
            if let row = context.rows[ref.task] {
                Button { context.onTask(row) } label: {
                    card(key: row.key, title: row.title, status: row.status.title, slice: ref.slice)
                }
                .buttonStyle(.plain)
                // A long press previews the card, with Open (ov-299).
                .taskKeyCard(row.key, speaksTitle: false)
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isButton)
                .accessibilityIdentifier("plan-card-\(row.key)")
            } else {
                let card = plan.card(ref.task)
                self.card(key: ref.key, title: card?.title ?? "", status: card?.status ?? "", slice: ref.slice)
                    .accessibilityElement(children: .combine)
            }
        }
    }

    private func card(key: String, title: String, status: String, slice: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(key).font(.system(.caption, design: .monospaced)).foregroundStyle(.secondary)
            Text(title).font(.subheadline).lineLimit(3)
            Text(slice.isEmpty ? status : "\(status) · \(slice)")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(.rect)
    }
}

// MARK: - Lane

/// A lane's page: why it exists, where it runs, its cards, what it spent,
/// its agents, and its timeline.
struct PlanLanePage: View {
    let lane: PlanLane
    let plan: PlanModel
    let record: PlanRecord?
    let context: PlanPageContext
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: PaneMetrics.step) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(lane.name).font(.title3.weight(.semibold))
                        Spacer()
                        Text(PlanWords.status(lane)).font(.subheadline).foregroundStyle(.secondary)
                    }
                    if !lane.reason.isEmpty { Text(lane.reason) }
                    if !place.isEmpty {
                        Text(place)
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                    if let stale = PlanWords.stale(lane, now: plan.nowMs) {
                        Label(stale, systemImage: "exclamationmark.circle.fill")
                            .font(.subheadline.weight(.medium))
                            .foregroundStyle(GlancePalette.amber(scheme))
                    }
                }
                .padding(.vertical, 2)
                .accessibilityIdentifier("plan-lane-page")
                if let theme = plan.theme(of: lane) {
                    Button { context.onOpen(.theme(theme.id)) } label: {
                        Label(theme.name, systemImage: "map")
                    }
                    .accessibilityHint("Opens the theme this lane serves")
                    .accessibilityIdentifier("plan-lane-theme")
                }
            }
            Section("Cards") { PlanCardRows(refs: lane.cards, plan: plan, context: context) }
            Section("Spend") {
                VStack(alignment: .leading, spacing: 4) {
                    Text("\(PlanWords.spend(lane.spend)) · \(PlanWords.fixRounds(lane.fixRounds))")
                    if let budget = PlanWords.budget(lane.spend, against: lane.budgetTokens) {
                        PlanBudgetLine(budget: budget, font: .footnote)
                    }
                    if lane.spend.totalTokens > 0, (lane.spend.costMicros ?? 0) > 0 {
                        Text(TaskUsageFormat.apiEquivalent).font(.footnote).foregroundStyle(.secondary)
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("plan-lane-spend")
            }
            if !lane.agents.isEmpty {
                Section("Agents") {
                    ForEach(Array(lane.agents.enumerated()), id: \.offset) { _, agent in
                        Text(PlanWords.agent(agent, now: plan.nowMs))
                    }
                }
            }
            if let record, !record.timeline.isEmpty {
                Section("Timeline") {
                    ForEach(record.timeline) { row in
                        HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.card) {
                            Text(Self.time(row.at))
                                .font(.footnote.monospacedDigit())
                                .foregroundStyle(.secondary)
                                .frame(width: 84, alignment: .leading)
                            Text(row.text).font(.subheadline)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
                .accessibilityIdentifier("plan-timeline")
            }
        }
        .listStyle(.insetGrouped)
    }

    /// ".claude/worktrees/mac-ux · mac-ux · Opus · in integ-9".
    private var place: String {
        [
            lane.worktreePath, lane.branch == lane.worktreePath ? "" : lane.branch, PlanWords.model(lane.model),
            lane.train.map { "in \($0)" } ?? "",
        ].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    /// "15:02" today; "Oct 3, 15:02" before.
    static func time(_ ms: Int64) -> String {
        let date = Date(timeIntervalSince1970: Double(ms) / 1000)
        let style: Date.FormatStyle =
            Calendar.current.isDateInToday(date)
            ? .dateTime.hour().minute() : .dateTime.month(.abbreviated).day().hour().minute()
        return date.formatted(style)
    }
}
