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
    let onOpen: (PlanPage) -> Void
    var defaults: UserDefaults = .standard
    /// Bumped when an entry folds, so the entries redraw from `defaults`.
    @State private var folds = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.boardMotionSlowdown) private var slowdown

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
            PlanNotice(title: PlanWords.nothingPlanned, detail: PlanWords.nothingPlannedDetail)
                .identified("plan-empty")
        } else {
            // What's running, then what's next (ov-298: the owner's order).
            if model.hasNow { now(model) }
            if !model.nextUp.isEmpty { nextUp(model) }
            if !model.shownThemes.isEmpty { themes(model) }
            // Behind `board_cost`: a runner without it sends no cost (ov-307).
            if let cost = model.cost, cost.isWorthShowing { costSection(cost) }
            // After Themes (ov-269 design 6.1): only on a runner with pages.
            if !plan.listedPages.isEmpty || plan.hiddenCount > 0 { pages() }
            let landed = model.landedToday()
            if !landed.isEmpty { landedToday(landed, model) }
        }
    }

    private func key(_ section: String) -> String { "board.plan.section.\(section).\(plan.host).\(plan.workspace.id)" }

    private func nextUp(_ model: PlanModel) -> some View {
        CollapsibleSection("Next Up", id: "plan.next", style: .navigator, key: key("next"), defaults: defaults,
            count: model.nextUp.count
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(model.nextUp.enumerated()), id: \.element.id) { index, lane in
                    PlanLaneRow(
                        lane: lane, theme: model.theme(of: lane), rank: index + 1, now: model.nowMs,
                        waitsOnOwner: false, selected: selected == .lane(lane.id), keyed: keyed
                    ) { onOpen(.lane(lane.id)) }
                    .changeWashed(lane.id)
                }
            }
            .listChanges(PlanChanges.lanes(model.nextUp, model, statuses: statuses))
        }
        .identified("plan-next-up")
    }

    /// Now (ov-309): each train not yet landed heads the lanes on it, and
    /// the lanes on none follow.
    private func now(_ model: PlanModel) -> some View {
        let groups = model.nowGroups
        return CollapsibleSection("Now", id: "plan.now", style: .navigator, key: key("now"), defaults: defaults,
            count: groups.reduce(0) { $0 + $1.lanes.count + ($1.train == nil ? 0 : 1) }
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(groups) { group in
                    if let train = group.train {
                        PlanTrainRow(train: train, ci: model.ci(of: train), now: model.nowMs)
                            .changeWashed(train.id)
                    }
                    ForEach(group.lanes) { lane in
                        PlanLaneRow(
                            lane: lane, theme: model.theme(of: lane), rank: nil, now: model.nowMs,
                            waitsOnOwner: model.waitsOnOwner(lane, statuses: statuses),
                            selected: selected == .lane(lane.id), keyed: keyed
                        ) { onOpen(.lane(lane.id)) }
                        .padding(.leading, group.train == nil ? 0 : NavigatorGrid.mark)
                        .changeWashed(lane.id)
                    }
                }
            }
            .listChanges(PlanChanges.now(model, statuses: statuses))
        }
        .identified("plan-now")
    }

    /// Themes (ov-331): each active theme a short brief, open until the
    /// owner closes it, in the board's order; paused and done ones folded into
    /// one row after them, and the work outside any theme as a footer.
    private func themes(_ model: PlanModel) -> some View {
        let shown = model.shownThemes
        let (active, closed) = (model.themeGroups.active, model.themeGroups.closed)
        return CollapsibleSection("Themes", id: "plan.themes", style: .navigator, key: key("themes"), defaults: defaults,
            count: shown.count,
            accessory: {
                if let summary = model.trackSummary(activity: activity) {
                    Text(summary)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .padding(.trailing, NavigatorGrid.gap * 2)
                        .probed("plan-themes-summary")
                }
            }
        ) {
            VStack(alignment: .leading, spacing: 0) {
                entries(active, model)
                if !closed.isEmpty { closedThemes(closed, model) }
                PlanOutsideRow(outside: model.outsideThemes(statuses: statuses), onOpen: onOpen)
            }
        }
        .identified("plan-themes")
    }

    private func entries(_ themes: [PlanTheme], _ model: PlanModel) -> some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            ForEach(themes) { theme in
                PlanThemeEntry(
                    theme: theme, plan: model, activity: activity, expanded: isExpanded(theme), selected: selected == .theme(theme.id),
                    keyed: keyed,
                    onToggle: { all in fold(theme, all: all, among: model.shownThemes) }, onOpen: onOpen
                )
                .changeWashed(theme.id)
            }
        }
        .listChanges(PlanChanges.entries(themes, model, activity: activity))
    }

    /// Paused and done, as one row that's closed until opened.
    private func closedThemes(_ themes: [PlanTheme], _ model: PlanModel) -> some View {
        CollapsibleSection("Paused and Done", id: "plan.themes.closed", style: .minor, key: key("themes-closed"), defaults: defaults,
            expandedByDefault: false, count: themes.count
        ) {
            entries(themes, model)
        }
        .padding(.top, NavigatorRhythm.group)
        .identified("plan-themes-closed")
    }

    private func isExpanded(_ theme: PlanTheme) -> Bool {
        _ = folds
        return PlanThemeFold.isExpanded(theme: theme, host: plan.host, workspace: plan.workspace.id, in: defaults)
    }

    /// Fold or unfold one entry on the shared spring, or, with ⌥, every theme
    /// to match it, as the Finder does (`PlanThemeFold.toggle`).
    private func fold(_ theme: PlanTheme, all: Bool, among themes: [PlanTheme]) {
        BoardMotion.toggle(reduceMotion: reduceMotion, slowedBy: slowdown) {
            PlanThemeFold.toggle(
                theme, all: all, among: themes, host: plan.host, workspace: plan.workspace.id, in: defaults)
            folds += 1
        }
    }

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
    /// The widest the canvas's plan reads (ov-331): about 75 characters of
    /// body text, close to a theme page's 720.
    static let readingWidth: CGFloat = 680
    /// The theme bar's height.
    static let bar: CGFloat = 4
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
                    HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                        Text(lane.heading)
                            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                            .layoutPriority(1)
                        Spacer(minLength: 0)
                        // Which theme it serves (the owner's ask, ov-273).
                        if let theme {
                            Text(theme.name)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.tail)
                                .probed("plan-lane-\(lane.name)-theme")
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
        .identified("plan-lane-\(lane.name)")
    }

    /// Next Up's reason; elsewhere, the state with its round, train or
    /// commit, and the cards.
    private var second: String {
        let slug = lane.slug.map { "\($0) · " } ?? ""
        if rank != nil { return slug + (lane.reason.isEmpty ? PlanWords.cards(lane.cards.count) : lane.reason) }
        return "\(slug)\(PlanWords.status(lane)) · \(PlanWords.cards(lane.cards.count))"
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

extension TaskBoardModel {
    /// Each card's last move in ms, by id: what keeps a theme from reading
    /// as quiet while its cards are still moving.
    var activity: [String: Int64] {
        Dictionary(
            rows.map { ($0.id, Int64($0.lastMoved.timeIntervalSince1970 * 1000)) }, uniquingKeysWith: { a, _ in a })
    }

    /// Each task's status by id: what the Plan view asks whether a lane
    /// waits on the owner.
    var statuses: [String: TaskStatus] {
        Dictionary(rows.map { ($0.id, $0.status) }, uniquingKeysWith: { a, _ in a })
    }
}
