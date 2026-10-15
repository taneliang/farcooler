import AgentKit
import SwiftUI

// The plan's overview (ov-268 design 6.1): the lanes working now, Next Up,
// the themes as short briefs, the orchestrator's pages and what landed today. A lane
// or a theme opens its page in the main area, where a task opens. Since
// ov-298 there's no Tasks | Plan control to show it in the navigator's place;
// the navigator lists the themes and the task index (`PlanNavigator`).
//
// Every list here moves as the task list does when the plan changes: a lane
// or theme that arrives or changes washes, and rows come, go and move on the
// shared spring (`listChanges`).

/// The plan's overview: a pane's content, so it scrolls in its host's own
/// scroll view and takes its inset.
struct PlanOverviewView: View {
    @ObservedObject var plan: PlanStore
    /// Each card's status as the board read it, by task id: whether a lane
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
            content
        }
        .padding(.bottom, Spacing.section)
        .frame(maxWidth: .infinity, alignment: .leading)
        .identified("plan-overview")
        .task(id: plan.generation) { await plan.reloadIfMoved() }
        .task(id: ObjectIdentifier(plan)) { await plan.readIfNeverRead() }
    }

    @ViewBuilder private var content: some View {
        let model = plan.plan
        if !plan.hasRead {
            if let trouble = plan.trouble {
                PlanNotice(title: trouble, detail: nil)
            } else {
                ProgressView().controlSize(.small)
                    .frame(maxWidth: .infinity, minHeight: NavigatorRhythm.placeholder)
            }
        } else if model.isEmpty && plan.pages.isEmpty && !(plan.keepsRulings && !model.rulings.isEmpty) {
            PlanEmptyNotice().identified("plan-empty")
        } else {
            // The sections, in the owner's order (ov-503, decision 7): Needs You
            // is the home's, above; Shipping, In Review, Building, Waiting,
            // Agents and Themes are `PlanSectionsView`'s; then what is
            // unchanged: Cost, Pages and Landed Today.
            let sections = plan.sections(needsYou: needsYou, statuses: statuses, activity: activity)
            PlanSectionsView(
                sections: sections, model: model, statuses: statuses, plan: plan, selected: selected, keyed: keyed,
                defaults: defaults, onOpen: onOpen)
            // Behind `board_cost`: a runner without it sends no cost (ov-307).
            if let cost = model.cost, cost.isWorthShowing { costSection(cost) }
            // After Themes (ov-269 design 6.1): only on a runner with pages.
            if !plan.listedPages.isEmpty || plan.hiddenCount > 0 { pages() }
            let landed = model.landedToday()
            if !landed.isEmpty { landedToday(landed, model) }
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

    private func landedToday(_ landed: [PlanLane], _ model: PlanModel) -> some View {
        CollapsibleSection("Landed Today", id: "plan.landed", style: .navigator, key: key("landed"), defaults: defaults,
            expandedByDefault: false, count: landed.count
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(landed) { lane in
                    PlanLaneRow(
                        lane: lane, theme: model.theme(of: lane), rank: nil, now: model.nowMs, waitsOnOwner: false,
                        selected: selected == .lane(lane.id), keyed: keyed
                    ) { onOpen(.lane(lane.id)) }
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
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if suggest.asked {
                Text(FirstRunCopy.Charter.suggestSent)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
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
                    .foregroundStyle(.secondary)
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
                    .foregroundStyle(.secondary)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.leading, NavigatorGrid.textInset)
        .padding(.vertical, NavigatorRhythm.card)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// One lane's row: its name and the theme it serves, then its reason or
/// state. In Next Up its rank sits in the glyph column; elsewhere its state's
/// glyph does, amber only when it's stale or waiting on the owner.
struct PlanLaneRow: View {
    let lane: PlanLane
    let theme: PlanTheme?
    /// Its place in Next Up, or nil.
    let rank: Int?
    let now: Int64
    let waitsOnOwner: Bool
    let selected: Bool
    let keyed: Bool
    /// The agent that works it, for the line under its title (ov-503).
    var agent: PlanFleetAgent?
    /// The landed train it still names, when its state doesn't already say.
    var landedTrain: String?
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    /// Over its token budget (ov-307), drawn beside whatever else it is: a lane
    /// that waits on the owner and is over budget says both.
    private var overBudget: String? { PlanWords.overBudget(lane) }

    /// Stale, waiting on the owner or over its token budget: the only lanes
    /// drawn in color.
    private var warning: String? {
        waitsOnOwner ? "Needs you" : PlanWords.stale(lane, now: now)
    }

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                mark.glyphColumn()
                VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                    // The theme sits beside the title when both fit on a line,
                    // else under it: a narrow column never squeezes either to
                    // an ellipsis.
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                            title.fixedSize(horizontal: true, vertical: false)
                            Spacer(minLength: 0)
                            themeName
                        }
                        VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                            title
                            themeName
                        }
                    }
                    // A key in the reason shows its card on hover (ov-299).
                    TaskKeyText(keysIn: second)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                    ForEach([warning, overBudget].compactMap { $0 }, id: \.self) { line in
                        Text(line)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                            .foregroundStyle(Tint.attention(scheme))
                            .probed(line == overBudget ? "plan-lane-\(lane.name)-over-budget" : "plan-lane-\(lane.name)-warning")
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .navigatorRow(selected: selected, keyed: keyed, leading: 0)
            .background {
                if hovering && !selected { RoundedRectangle.control.fill(Fill.hover).boxOutset() }
            }
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibility)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .help(lane.slug ?? "")
        .identified("plan-lane-\(lane.name)")
    }

    private var title: some View {
        Text(lane.heading)
            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
    }

    /// Which theme it serves (the owner's ask, ov-273).
    @ViewBuilder private var themeName: some View {
        if let theme {
            Text(theme.name)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .probed("plan-lane-\(lane.name)-theme")
        }
    }

    /// Next Up's reason; elsewhere, the state with its round, train or
    /// commit, and the cards.
    private var second: String {
        if rank != nil { return lane.reason.isEmpty ? PlanWords.cards(lane.cards.count) : lane.reason }
        return Self.second(lane, agent: agent, landedTrain: landedTrain, now: now, aged: warning == nil)
    }

    /// "Building · ov-481, ov-482 · Builder · Sonnet · 380K · 3 h": the state
    /// with its round and train, the cards (their count past three), the main
    /// agent and its tokens, and how long it has been in this state.
    static func second(_ lane: PlanLane, agent: PlanFleetAgent?, landedTrain: String?, now: Int64, aged: Bool = true) -> String {
        PlanWords.laneSecond(lane, agent: agent, landedTrain: landedTrain, now: now, aged: aged)
    }

    @ViewBuilder private var mark: some View {
        if let rank {
            Text("\(rank)")
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold).monospacedDigit())
                .foregroundStyle(.secondary)
        } else {
            Image(systemName: PlanGlyph.name(lane.state))
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(warning == nil && overBudget == nil ? AnyShapeStyle(.secondary) : AnyShapeStyle(Tint.attention(scheme)))
                .accessibilityHidden(true)
        }
    }

    private var accessibility: String {
        var parts = [rank.map { "\(PlanWords.ordinal($0)) up" }, lane.heading, theme?.name, second, warning, PlanWords.overBudgetSpoken(lane)]
        parts.removeAll { $0 == nil }
        return parts.compactMap { $0 }.joined(separator: ", ")
    }
}

/// A lane state's glyph: neutral, a shape to scan by.
enum PlanGlyph {
    static func name(_ state: LaneState) -> String {
        switch state {
        case .queued: "clock"
        case .building: "hammer"
        case .review: "eye"
        case .fixing: "wrench.and.screwdriver"
        case .landing: "arrow.down.to.line"
        case .landed: "checkmark.circle"
        case .dropped: "xmark.circle"
        case .unknown: "questionmark.circle"
        }
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
        case .done: AnyShapeStyle(.secondary)
        case .inReview: AnyShapeStyle(.tertiary)
        case .inProgress: AnyShapeStyle(.quaternary)
        case .notStarted: AnyShapeStyle(.quinary)
        }
    }
}
