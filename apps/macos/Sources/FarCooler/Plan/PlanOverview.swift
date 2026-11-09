import AgentKit
import SwiftUI

// The plan's overview (ov-268 design 6.1): the tickets working now, Next Up,
// the themes as short briefs, the orchestrator's pages and what landed today. A ticket
// or a theme opens its page in the main area, where a task opens. Since
// ov-298 there's no Tasks | Plan control to show it in the navigator's place;
// the navigator lists the themes and the task index (`PlanNavigator`).
//
// Every list here moves as the task list does when the plan changes: a ticket
// or theme that arrives or changes washes, and rows come, go and move on the
// shared spring (`listChanges`).

/// The plan's overview: a pane's content, so it scrolls in its host's own
/// scroll view and takes its inset.
struct PlanOverviewView: View {
    @ObservedObject var plan: PlanStore
    /// Each card's status as the board read it, by task id: whether a ticket
    /// is waiting on the owner.
    let statuses: [String: TaskStatus]
    /// The page open in the main area, drawn selected.
    let selected: PlanPage?
    /// Whether the navigator has the keyboard: a selected row reads in the
    /// accent.
    var keyed = false
    /// Each card's last move on the board, by task id (the theme's "quiet").
    var activity: [String: Int64] = [:]
    /// The workspace's Needs You count, for the glance band and the sections
    /// that say it (ov-503).
    var needsYou = 0
    let onOpen: (PlanPage) -> Void
    var defaults: UserDefaults = .standard

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.section) {
            // The plan in hand stays; this says why it may be a little old.
            if plan.hasRead, plan.reconnecting { PlanReconnecting() }
            if plan.hasRead, let line = plan.staleLine { PlanStale(line: line) { Task { await plan.tryAgain() } } }
            content
        }
        .padding(.bottom, Spacing.section)
        .frame(maxWidth: .infinity, alignment: .leading)
        .identified("plan-overview")
        .task(id: plan.generation) { await plan.reloadIfMoved() }
        .task(id: ObjectIdentifier(plan)) { await plan.readIfNeverRead() }
        // Asking again for a plan that didn't answer stops when nobody is looking at it.
        .onAppear { plan.attach() }
        .onDisappear { plan.detach() }
    }

    @ViewBuilder private var content: some View {
        let model = plan.plan
        if !plan.hasRead {
            if plan.reconnecting {
                PlanReconnecting()
            } else if let trouble = plan.trouble {
                VStack(alignment: .leading, spacing: 0) {
                    PlanNotice(title: trouble, detail: nil)
                    Button("Try Again") { Task { await plan.tryAgain() } }
                        .padding(.leading, NavigatorGrid.textInset)
                }
            } else {
                ProgressView().controlSize(.small)
                    .frame(maxWidth: .infinity, minHeight: NavigatorRhythm.placeholder)
            }
        } else if model.isEmpty && plan.pages.isEmpty && !(plan.keepsRulings && !model.rulings.isEmpty) {
            PlanEmptyNotice().identified("plan-empty")
        } else {
            // The sections, in the owner's order (ov-503, decision 7): Needs You
            // is the home's, above; Landing, In Review, Building, Waiting,
            // Agents and Themes are `PlanSectionsView`'s; then what is
            // unchanged: Cost, Pages and Landed Today.
            let view = plan.view(needsYou: needsYou, statuses: statuses, activity: activity)
            if let view {
                PlanSectionsView(
                    view: view, model: model, statuses: statuses, plan: plan, selected: selected, keyed: keyed,
                    defaults: defaults, onOpen: onOpen)
            } else {
                // The core couldn't read the plan: say so, as Android and the web do.
                PlanNotice(title: PlanWords.couldntRead, detail: nil).identified("plan-unreadable")
            }
            // Behind `board_cost`: a runner without it sends no cost (ov-307).
            if let cost = model.cost, cost.isWorthShowing { costSection(cost) }
            // After Themes (ov-269 design 6.1): only on a runner with pages.
            if !plan.listedPages.isEmpty || plan.hiddenCount > 0 { pages() }
            if let view, let landed = view.section(.landedToday) { landedToday(landed, view) }
        }
    }

    private func key(_ section: String) -> String { "board.plan.section.\(section).\(plan.host).\(plan.workspace.id)" }

    /// The runner's week and cost per finished card by harness and model.
    private func costSection(_ cost: PlanCostRead) -> some View {
        CollapsibleSection("Cost", id: "plan.cost", style: .navigator, key: key("cost"), defaults: defaults
        ) {
            PlanCostBlock(
                cost: cost, bodyFont: .system(size: WorkspaceStyle.PaneText.body),
                secondaryFont: .system(size: WorkspaceStyle.PaneText.secondary)
            )
            .padding(.leading, NavigatorGrid.textInset)
            .padding(.vertical, NavigatorRhythm.card)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .identified("plan-cost-section")
    }

    private func pages() -> some View {
        let listed = plan.listedPages
        return CollapsibleSection("Pages", id: "plan.pages", style: .navigator, key: key("pages"), defaults: defaults,
            count: listed.count
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(listed) { page in
                    PlanPageRow(
                        page: page, now: Self.nowMs(), selected: selected == .page(page.slot), keyed: keyed,
                        onHide: { plan.hide(page.slot) }
                    ) { onOpen(.page(page.slot)) }
                }
                if plan.hiddenCount > 0 {
                    Button("Show Hidden Pages") { plan.showHiddenPages() }
                        .buttonStyle(.link)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .padding(.leading, NavigatorGrid.textInset)
                        .padding(.top, NavigatorRhythm.air)
                        .identified("plan-show-hidden-pages")
                }
            }
        }
        .identified("plan-pages")
    }

    /// Now, for "Updated 12 min ago": a page's own clock is the runner's
    /// stamp, and the Mac's is close enough for minutes.
    static func nowMs() -> Int64 { Int64(Date().timeIntervalSince1970 * 1000) }

    /// Landed on the runner's day: the view's cards, under the view's title.
    private func landedToday(_ section: PlanView.Section, _ view: PlanView) -> some View {
        CollapsibleSection(section.title, id: "plan.landed", style: .navigator, key: key("landed"), defaults: defaults,
            expandedByDefault: !section.folded, count: section.count
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(view.landedToday) { card in
                    PlanCardRow(card: card, selected: selected == .ticket(card.id), keyed: keyed) { onOpen(.ticket(card.id)) }
                }
            }
        }
        .identified("plan-landed")
    }
}

/// Sizes the Plan view's own shapes use, beside the navigator's grid.
enum PlanMetrics {
    /// The widest the canvas's plan draws. The 680-pt reading width went with
    /// the prose briefs (ov-503); this only stops a very wide display from
    /// stretching a row from its title to its theme across a whole screen.
    static let widest: CGFloat = 1600
    /// The canvas width from which the sections split into two columns, and the
    /// gutters the home sets either side of its content.
    static let twoColumnCanvas: CGFloat = 960
    static var gutter: CGFloat { ColumnGrid.a + ColumnGrid.step }
    /// The content width that is `twoColumnCanvas` once the gutters and the
    /// card's own inset are in.
    static var twoColumns: CGFloat { twoColumnCanvas - 2 * gutter - 2 * ColumnGrid.a }
    /// The room between the two columns.
    static let columnGap: CGFloat = 2 * ColumnGrid.step
    /// The narrowest a theme's tile draws.
    static let tile: CGFloat = 240
    /// The theme bar's height.
    static let bar: CGFloat = 4
}

/// "Nothing is planned on this board yet.", and what to do about it (ov-519):
/// with no orchestrator, what it's for and Start Orchestrator; with one, to
/// tell it what to build. Without a workspace to start one in (a runner that
/// keeps no workspaces), what the plan always said.
struct PlanEmptyNotice: View {
    @Environment(\.orchestratorInvite) private var invite
    @Environment(\.besideOrchestratorColumn) private var beside

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.card) {
            PlanNotice(
                title: PlanWords.nothingPlanned,
                detail: invite == nil
                    ? PlanWords.nothingPlannedDetail
                    : invite?.seated == true ? PlanWords.nothingPlannedRunning : PlanWords.nothingPlannedStart)
            if let invite, !invite.seated, invite.canAct, !beside {
                StartOrchestratorMenu(invite: invite)
                    .padding(.leading, NavigatorGrid.textInset)
            }
            if let suggest = invite?.suggestThemes {
                SuggestThemesNotice(suggest: suggest)
                    .padding(.leading, NavigatorGrid.textInset)
            }
        }
    }
}

/// Under an empty plan with the orchestrator running (ov-519): a way to
/// begin for someone who doesn't know what to ask for. It sends the runner's
/// prompt through the orchestrator's chat; the themes come back there.
struct SuggestThemesNotice: View {
    let suggest: SuggestThemes

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.air) {
            Text(FirstRunCopy.Charter.suggestTitle)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
            Text(FirstRunCopy.Charter.suggestBody)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.quietInk)
                .fixedSize(horizontal: false, vertical: true)
            if suggest.asked {
                Text(FirstRunCopy.Charter.suggestSent)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.quietInk)
                    .identified("plan-themes-asked")
            } else {
                Button(FirstRunCopy.Charter.suggestButton, action: suggest.send)
                    .identified("plan-suggest-themes")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .identified("plan-suggest")
    }
}

/// Under a Plan section that says it's empty, the step that fills it
/// (ov-519): the orchestrator is where work is queued, so with one it's a way
/// to it, and without one, Start Orchestrator.
struct PlanNextStep: View {
    @Environment(\.orchestratorInvite) private var invite
    @Environment(\.besideOrchestratorColumn) private var beside

    var body: some View {
        if let invite, invite.canAct {
            VStack(alignment: .leading, spacing: NavigatorRhythm.air) {
                Text(invite.seated ? PlanWords.nothingQueuedNext : PlanWords.nothingQueuedStart)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.quietInk)
                if beside {
                    // The column beside it is where to tell the orchestrator.
                } else if invite.seated {
                    Button(FirstRunCopy.NeedsYou.showOrchestrator, action: invite.onShow)
                        .buttonStyle(.link)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                } else {
                    StartOrchestratorMenu(invite: invite, caption: false)
                }
            }
            .padding(.leading, NavigatorGrid.textInset)
            .padding(.bottom, NavigatorRhythm.air)
            .frame(maxWidth: .infinity, alignment: .leading)
            .identified("plan-next-step")
        }
    }
}

/// A read that got no answer (ov-631): the runner is restarting or the link
/// is down. Not a broken plan, and it reads again by itself.
struct PlanReconnecting: View {
    var body: some View {
        HStack(spacing: NavigatorRhythm.lineGap) {
            ProgressView().controlSize(.small)
            Text(PlanWords.reconnecting)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.quietInk)
        }
        .padding(.leading, NavigatorGrid.textInset)
        .padding(.vertical, NavigatorRhythm.card)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .identified("plan-reconnecting")
    }
}

/// A plan in hand that the runner can no longer be asked about (ov-662): a
/// quiet line over it, and Try Again.
struct PlanStale: View {
    let line: String
    let tryAgain: () -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: NavigatorRhythm.lineGap) {
            Text(line)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.quietInk)
                .fixedSize(horizontal: false, vertical: true)
            Button("Try Again", action: tryAgain)
                .buttonStyle(.link)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
        }
        .padding(.leading, NavigatorGrid.textInset)
        .padding(.vertical, NavigatorRhythm.card)
        .frame(maxWidth: .infinity, alignment: .leading)
        .identified("plan-stale")
    }
}

/// "Nothing is planned on this board yet.", and why there would be.
struct PlanNotice: View {
    let title: String
    let detail: String?

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
            Text(title)
                .font(.system(size: WorkspaceStyle.PaneText.body, weight: .semibold))
            if let detail {
                Text(detail)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.quietInk)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.leading, NavigatorGrid.textInset)
        .padding(.vertical, NavigatorRhythm.card)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// "● Needs you: …": the one colored mark a theme has, with words.
struct PlanAsk: View {
    let text: String
    var lines: Int? = nil
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.tight + 2) {
            Circle()
                .fill(Tint.attention(scheme))
                .frame(width: 6, height: 6)
                .accessibilityHidden(true)
            (Text("Needs you: ").fontWeight(.medium).foregroundStyle(Tint.attention(scheme)) + Text(text))
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .lineLimit(lines)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.top, Spacing.tight)
        .identified("plan-ask")
    }
}

/// A theme's cards by status, as one bar of neutral parts: done darkest,
/// then in review, in progress, and what hasn't started on the track.
struct PlanBar: View {
    let counts: PlanCounts

    var body: some View {
        let parts = PlanWords.segments(counts)
        let total = max(1, PlanWords.total(counts))
        GeometryReader { proxy in
            HStack(spacing: 1) {
                ForEach(Array(parts.enumerated()), id: \.offset) { _, part in
                    Rectangle()
                        .fill(Self.style(part.kind))
                        .frame(width: max(0, proxy.size.width * CGFloat(part.count) / CGFloat(total) - 1))
                }
                Spacer(minLength: 0)
            }
            .clipShape(Capsule())
        }
        .frame(height: PlanMetrics.bar)
        .background(Capsule().fill(Fill.inset()))
        .accessibilityElement()
        .accessibilityLabel(PlanWords.breakdown(counts))
    }

    static func style(_ kind: PlanSegment.Kind) -> AnyShapeStyle {
        switch kind {
        case .done: AnyShapeStyle(.secondary)  // style-exempt: a bar segment is a fill, not text
        case .inReview: AnyShapeStyle(.tertiary)  // style-exempt: a bar segment is a fill, not text
        case .inProgress: AnyShapeStyle(.quaternary)
        case .notStarted: AnyShapeStyle(.quinary)
        }
    }
}
