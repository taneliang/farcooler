import AgentKit
import SwiftUI

// A theme's page and a lane's page (ov-268 design 6.2 and 6.3), opened in the
// main area from the Plan view, as a task opens from the board. Laid out as
// the History page is: a document column, a title, and sections under small
// headings.

/// What a plan page needs from the board: its tasks, to draw a card as the
/// board draws it, and where a task, a lane or a theme opens.
struct PlanPageContext {
    /// The board's rows by task id.
    var rows: [String: TaskRow]
    var onTask: (TaskRow) -> Void
    var onOpen: (PlanPage) -> Void
    /// The board's worktrees, with their terminals: what an orchestrator's
    /// page names by `worktree` and `terminal` (ov-284).
    var worktrees: [Worktree] = []
    /// Where a reference on an orchestrator's page opens.
    var onDestination: (PageDestination) -> Void = { _ in }
}

/// The page for `page`, or why there's none.
struct PlanPageView: View {
    @ObservedObject var plan: PlanStore
    let page: PlanPage
    let context: PlanPageContext

    var body: some View {
        Group {
            switch page {
            case .theme(let id):
                if let theme = plan.theme(id) {
                    PlanThemePage(
                        theme: theme, plan: plan.plan, record: plan.records[page], context: context,
                        anchored: plan.anchoredPages(to: id), world: context.world(plan), onHide: { plan.hide($0) })
                } else {
                    missing("Theme Not Found")
                }
            case .lane(let id):
                if let lane = plan.lane(id) {
                    PlanLanePage(lane: lane, plan: plan.plan, record: plan.records[page], context: context)
                } else {
                    missing("Lane Not Found")
                }
            case .page(let slot):
                if let found = plan.page(slot) {
                    PlanOrchestratorPage(page: found, world: context.world(plan), context: context)
                } else if let trouble = plan.pagesTrouble, !plan.reading {
                    ContentUnavailableView {
                        Label(trouble, systemImage: "doc.text")
                    } actions: {
                        Button("Try Again") { Task { await plan.reload() } }
                    }
                    .identified("plan-pages-unavailable")
                } else if plan.pagesRead || !plan.pagesAvailable {
                    // Removed, or never published: said once, never a spinner.
                    ContentUnavailableView("Page Not Found", systemImage: "doc.text")
                        .identified("plan-page-not-found")
                } else {
                    missing("Page Not Found")
                }
            case .needsYou:
                // Drawn by the window, which holds the answers
                // (`PlanNeedsYouPage`); never reached through here.
                missing("Needs You")
            }
        }
        // One card for every page and every state it can be in (ov-297).
        .contentCard()
        .task(id: page) {
            await plan.readIfNeverRead()
            await plan.readRecord(page)
        }
        .task(id: plan.generation) { await plan.reloadIfMoved() }
    }

    /// The page with nothing to show: not found once the plan is read; until
    /// then a spinner, and if the read failed (a runner that can't answer,
    /// once the CLI's own timeout has passed), the unavailable state with
    /// Try Again, never the spinner forever.
    private func missing(_ title: String) -> some View {
        Group {
            if plan.hasRead {
                ContentUnavailableView(title, systemImage: "map")
            } else if let trouble = plan.trouble, !plan.reading {
                ContentUnavailableView {
                    Label(trouble, systemImage: "map")
                } actions: {
                    Button("Try Again") { Task { await plan.reload() } }
                }
                .identified("plan-page-unavailable")
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                    .identified("plan-page-reading")
            }
        }
    }
}

/// The page's column: a document, flat on the page's card as the History page
/// is (ov-220), not a card of its own.
struct PlanDocument<Content: View>: View {
    let id: String
    @ViewBuilder let content: () -> Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.section + Spacing.group) {
                content()
            }
            .frame(maxWidth: 720, alignment: .leading)
            .padding(.horizontal, ColumnGrid.a + ColumnGrid.step)
            .padding(.vertical, 2 * ColumnGrid.rhythm)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .identified(id)
    }
}

/// A page's title, with its state trailing.
private struct PlanTitle: View {
    let title: String
    let state: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: SidebarGrid.gap) {
            Text(title)
                .font(.system(size: 17, weight: .semibold))
                .textSelection(.enabled)
            Spacer(minLength: SidebarGrid.gap)
            Text(state)
                .font(.system(size: WorkspaceStyle.PaneText.body))
                .foregroundStyle(.secondary)
        }
    }
}

/// A section: its small heading, an accessory trailing, then its content.
struct PlanSection<Accessory: View, Content: View>: View {
    let title: String
    @ViewBuilder var accessory: () -> Accessory
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            HStack(alignment: .firstTextBaseline, spacing: SidebarGrid.gap) {
                SectionTitle(text: title, style: .navigator)
                Spacer(minLength: SidebarGrid.gap)
                accessory()
            }
            content()
        }
    }
}

extension PlanSection where Accessory == EmptyView {
    init(title: String, @ViewBuilder content: @escaping () -> Content) {
        self.init(title: title, accessory: { EmptyView() }, content: content)
    }
}

/// Body text on a plan page.
private extension Text {
    func planBody() -> some View {
        font(.system(size: WorkspaceStyle.PaneText.title))
            .lineSpacing(2)
            .fixedSize(horizontal: false, vertical: true)
            .textSelection(.enabled)
    }
}

// MARK: - Theme

/// A theme's page: its outcome, where it stands and what changed, what's
/// next and what needs the owner, its lanes, and its cards as the board
/// draws them.
struct PlanThemePage: View {
    let theme: PlanTheme
    let plan: PlanModel
    let record: PlanRecord?
    let context: PlanPageContext
    /// The orchestrator's pages anchored to this theme (ov-284).
    var anchored: [BoardPage]
    var world: PageWorld
    var onHide: (String) -> Void
    @State private var showingChange: Bool

    init(
        theme: PlanTheme, plan: PlanModel, record: PlanRecord?, context: PlanPageContext, showingChange: Bool = false,
        anchored: [BoardPage] = [], world: PageWorld = PageWorld(), onHide: @escaping (String) -> Void = { _ in }
    ) {
        self.theme = theme
        self.plan = plan
        self.record = record
        self.context = context
        self.anchored = anchored
        self.world = world
        self.onHide = onHide
        _showingChange = State(initialValue: showingChange)
    }

    var body: some View {
        PlanDocument(id: "plan-theme-page") {
            VStack(alignment: .leading, spacing: Spacing.group) {
                PlanTitle(title: theme.name, state: theme.state.capitalized)
                if !theme.outcome.isEmpty {
                    Text(theme.outcome).planBody().foregroundStyle(.secondary)
                }
                PlanBar(counts: theme.counts).padding(.top, Spacing.tight)
            }
            story
            // Where it stands, then what it needs from the owner, then Next:
            // the order the canvas's entry reads in (ov-331), so opening the
            // page reads as the rest of the entry.
            if !theme.next.isEmpty || !theme.ownerAsk.isEmpty {
                VStack(alignment: .leading, spacing: Spacing.group) {
                    if !theme.ownerAsk.isEmpty {
                        PlanAsk(text: theme.ownerAsk)
                            .font(.system(size: WorkspaceStyle.PaneText.title))
                    }
                    if !theme.next.isEmpty {
                        PlanSection(title: "Next") { Text(theme.next).planBody() }
                            .identified("plan-theme-next")
                    }
                }
            }
            // After Needs You and before Lanes (ov-269 design 6.1).
            PlanAnchoredPages(pages: anchored, world: world, context: context, onHide: onHide)
            decided
            if PlanThemeSpend.hasSomething(theme) {
                PlanSection(title: "Spend") {
                    PlanThemeSpend(
                        theme: theme, bodyFont: .system(size: WorkspaceStyle.PaneText.title),
                        secondaryFont: .system(size: WorkspaceStyle.PaneText.secondary))
                }
            }
            lanes
            cards
        }
    }

    /// The calls made for the owner that name this theme (ov-331): the canvas's
    /// entry shows two, and the page all of them, standing first.
    @ViewBuilder private var decided: some View {
        let rulings = plan.rulings(in: theme)
        if !rulings.isEmpty {
            PlanSection(title: "Decided") {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(rulings.filter(\.isStanding)) { PlanRulingRow(ruling: $0, copy: PlanRulingsList.toPasteboard) }
                    ForEach(rulings.filter { !$0.isStanding }) { PlanSettledRulingRow(ruling: $0, copy: PlanRulingsList.toPasteboard) }
                }
            }
            .identified("plan-theme-decided")
        }
    }

    @ViewBuilder private var story: some View {
        PlanSection(title: "Where It Stands") {
            HStack(spacing: SidebarGrid.gap) {
                if theme.storyAt > 0 {
                    Text("Updated \(PlanWords.ago(theme.storyAt, now: plan.nowMs))")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                }
                if record?.previousStory != nil {
                    Button(showingChange ? "Hide Changes" : "What Changed") { showingChange.toggle() }
                        .buttonStyle(.link)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .identified("plan-what-changed")
                }
            }
        } content: {
            if theme.story.isEmpty {
                Text("The orchestrator hasn’t written where this stands yet.")
                    .planBody().foregroundStyle(.secondary)
            } else {
                Text(theme.story).planBody()
            }
            if showingChange, let before = record?.previousStory {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    Text("Before, \(PlanWords.ago(before.at, now: plan.nowMs))")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                        .foregroundStyle(.secondary)
                    Text(before.story).planBody().foregroundStyle(.secondary)
                }
                .padding(Spacing.inset)
                .frame(maxWidth: .infinity, alignment: .leading)
                .surface(.inset, in: .card)
                .identified("plan-previous-story")
            }
        }
    }

    @ViewBuilder private var lanes: some View {
        let lanes = plan.lanes(in: theme)
        if !lanes.isEmpty {
            PlanSection(title: "Lanes") {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(lanes) { lane in
                        PlanPageLaneRow(lane: lane) { context.onOpen(.lane(lane.id)) }
                    }
                }
            }
        }
    }

    @ViewBuilder private var cards: some View {
        PlanSection(title: "Cards") {
            Text(PlanWords.breakdown(theme.counts))
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
        } content: {
            PlanCardList(refs: theme.cards, plan: plan, context: context)
        }
    }
}

/// A lane on a theme's page: its name, its state, and its cards' keys.
private struct PlanPageLaneRow: View {
    let lane: PlanLane
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.inset) {
                Image(systemName: PlanGlyph.name(lane.state))
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
                    .frame(width: ColumnGrid.step)
                Text(lane.heading)
                    .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .layoutPriority(1)
                Text(PlanWords.status(lane))
                    .font(.system(size: WorkspaceStyle.PaneText.body))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer(minLength: Spacing.group)
                // Each key shows its card on hover (ov-299).
                TaskKeyText(keysIn: lane.cards.map(\.key).joined(separator: " "))
                    .font(TaskKeyColumn.font)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            .padding(.vertical, NavigatorRhythm.air)
            .padding(.horizontal, NavigatorGrid.outset)
            .background {
                if hovering { RoundedRectangle.control.fill(Fill.hover) }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .padding(.horizontal, -NavigatorGrid.outset)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .identified("plan-page-lane-\(lane.name)")
    }
}

/// Cards as the board draws them (`CompactTaskRow` with the board's own
/// status line), from the board's rows; a card the board hasn't read draws
/// from the plan's read of it.
private struct PlanCardList: View {
    let refs: [PlanCardRef]
    let plan: PlanModel
    let context: PlanPageContext

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(refs, id: \.self) { ref in
                if let row = context.rows[ref.task] {
                    CompactTaskRow(key: row.key, title: row.title) {
                        HStack(spacing: Spacing.tight) {
                            Text(row.status.title)
                            if !ref.slice.isEmpty { Text("· \(ref.slice)") }
                        }
                        .font(.system(size: WorkspaceStyle.PaneText.minimum))
                        .foregroundStyle(.secondary)
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { context.onTask(row) }
                    .accessibilityElement(children: .combine)
                    .accessibilityAddTraits(.isButton)
                    .identified("plan-card-\(row.key)")
                } else {
                    let card = plan.card(ref.task)
                    CompactTaskRow(key: ref.key, title: card?.title ?? "") {
                        Text(card?.status ?? "")
                            .font(.system(size: WorkspaceStyle.PaneText.minimum))
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
        .environment(\.taskKeyWidth, TaskKeyColumn.width(for: refs.map(\.key)))
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

    var body: some View {
        PlanDocument(id: "plan-lane-page") {
            VStack(alignment: .leading, spacing: Spacing.group) {
                PlanTitle(title: lane.heading, state: PlanWords.status(lane))
                if !lane.reason.isEmpty { Text(lane.reason).planBody() }
                if !place.isEmpty {
                    Text(place)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                if let theme = plan.theme(of: lane) {
                    Button { context.onOpen(.theme(theme.id)) } label: {
                        Label(theme.name, systemImage: "map").labelStyle(.titleAndIcon)
                    }
                    .buttonStyle(.link)
                    .font(.system(size: WorkspaceStyle.PaneText.body))
                    .help("Open the theme this lane serves")
                    .identified("plan-lane-theme")
                }
                if let stale = PlanWords.stale(lane, now: plan.nowMs) {
                    StaleLine(text: stale)
                }
            }
            PlanSection(title: "Cards") {
                PlanCardList(refs: lane.cards, plan: plan, context: context)
            }
            PlanSection(title: "Spend") {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    Text("\(PlanWords.spend(lane.spend)) · \(PlanWords.fixRounds(lane.fixRounds))").planBody()
                    if let budget = PlanWords.budget(lane.spend, against: lane.budgetTokens) {
                        PlanBudgetLine(budget: budget, font: .system(size: WorkspaceStyle.PaneText.secondary))
                    }
                    if lane.spend.totalTokens > 0, (lane.spend.costMicros ?? 0) > 0 {
                        Text(TaskUsageFormat.apiEquivalent)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            if !lane.agents.isEmpty {
                PlanSection(title: "Agents") {
                    VStack(alignment: .leading, spacing: Spacing.tight) {
                        ForEach(Array(lane.agents.enumerated()), id: \.offset) { _, agent in
                            Text(PlanWords.agent(agent, now: plan.nowMs)).planBody()
                        }
                    }
                }
            }
            if let record, !record.timeline.isEmpty {
                PlanSection(title: "Timeline") {
                    PlanTimeline(rows: record.timeline)
                }
            }
        }
    }

    /// ".claude/worktrees/mac-ux · mac-ux · Opus · in integ-9".
    private var place: String {
        [
            lane.slug ?? "", lane.worktreePath, lane.branch == lane.worktreePath ? "" : lane.branch, PlanWords.model(lane.model),
            lane.train.map { "in \($0)" } ?? "",
        ].filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

/// A stale lane's warning on its page: amber words, with their glyph.
private struct StaleLine: View {
    let text: String
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Label(text, systemImage: "exclamationmark.circle.fill")
            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
            .foregroundStyle(Tint.attention(scheme))
    }
}

/// A lane's record, oldest first: the time, then what happened.
private struct PlanTimeline: View {
    let rows: [PlanTimelineRow]

    var body: some View {
        Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: Spacing.inset, verticalSpacing: Spacing.tight) {
            ForEach(rows) { row in
                GridRow {
                    Text(Self.time(row.at))
                        .font(.system(size: WorkspaceStyle.PaneText.secondary).monospacedDigit())
                        .foregroundStyle(.secondary)
                    Text(row.text)
                        .font(.system(size: WorkspaceStyle.PaneText.body))
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .identified("plan-timeline")
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

// MARK: - The task page's line

/// "In lane mac-ux · Visual language" under a task's title, on a runner
/// that keeps a plan (6.4): each half opens its page. Read from the plan the
/// window already holds; nothing new is asked of the runner.
struct PlanTaskLineView: View {
    @ObservedObject var plan: PlanStore
    let task: String
    let onOpen: (PlanPage) -> Void

    var body: some View {
        if plan.available, let line = plan.plan.taskLine(task) {
            HStack(spacing: Spacing.tight) {
                if let lane = line.lane, let words = line.laneWords {
                    Button(words) { onOpen(.lane(lane.id)) }
                        .help("Open lane \(lane.heading)")
                }
                if line.lane != nil, line.theme != nil {
                    Text("·").foregroundStyle(.secondary)
                }
                if let theme = line.theme {
                    Button(theme.name) { onOpen(.theme(theme.id)) }
                        .help("Open theme \(theme.name)")
                }
            }
            .buttonStyle(.link)
            .font(TaskTypography.meta)
            .accessibilityElement(children: .contain)
            .accessibilityLabel(line.text)
            .identified("plan-task-line")
            // Under the header's key, on the task card's paper: no fill of its
            // own, since the header and tab bar draw none (ov-223).
            .padding(.horizontal, TaskTypography.inset.leading)
            .padding(.bottom, ColumnGrid.rhythm)
            .frame(maxWidth: .infinity, alignment: .leading)
        } else if plan.available {
            Color.clear.frame(height: 0)
                .task(id: ObjectIdentifier(plan)) { await plan.readIfNeverRead() }
        }
    }
}
