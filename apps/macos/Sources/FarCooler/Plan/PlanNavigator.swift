import AgentKit
import SwiftUI

// The navigator of a board with a plan (ov-298). The owner, Oct 4: "we
// basically have 3 levels of planning -- themes, lanes, tasks and they should
// be treated separately", and "Let's make that the primary way of working.
// Can you remove the Tasks and Plan toggle". The navigator is navigation,
// ordered by that hierarchy, every section a peer pane:
//
//   the orchestrator's row, one line;
//   Themes, a row each with its progress, opening the theme's page, where
//   its lanes and cards are;
//   Pages, the orchestrator's own, when it has published any;
//   Tasks, the status groups as an index, collapsed until opened;
//   Terminals and Worktrees, as before.
//
// A runner without `board_plan`, or a board with nothing planned yet, draws
// the task list it always did (`PlanStore.planned`).

/// What the navigator of a planned board draws, as values.
enum PlanNavigator {
    /// Where a planned board's open status groups are kept on this Mac:
    /// apart from the task list's, which a board with no plan still uses.
    static func collapsedKey(host: String, workspace: String) -> String {
        "board.plan.collapsed.\(host).\(workspace)"
    }

    /// The task index's closed groups: every one until the person opens one.
    static func collapsed(host: String, workspace: String, from defaults: UserDefaults) -> Set<TaskStatus> {
        guard let words = defaults.stringArray(forKey: collapsedKey(host: host, workspace: workspace)) else {
            return Set(TaskBoardModel.order)
        }
        return Set(words.compactMap(TaskStatus.init(rawValue:)))
    }

    static func setCollapsed(_ statuses: Set<TaskStatus>, host: String, workspace: String, in defaults: UserDefaults) {
        defaults.set(statuses.map(\.rawValue).sorted(), forKey: collapsedKey(host: host, workspace: workspace))
    }

    /// The themes listed: the plan's shown ones, narrowed by the filter.
    static func themes(_ model: PlanModel, filter: String) -> [PlanTheme] {
        model.shownThemes.filter { BoardFilter.isEmpty(filter) || BoardFilter.matches(key: "", title: $0.name, filter) }
    }

    /// The orchestrator's pages listed, narrowed by the filter.
    static func pages(_ listed: [BoardPage], filter: String) -> [BoardPage] {
        listed.filter { BoardFilter.isEmpty(filter) || BoardFilter.matches(key: "", title: $0.title, filter) }
    }

    /// A theme's progress, as its row says it: "3/10", the cards done of
    /// those counted.
    static func progress(_ counts: PlanCounts) -> String { "\(counts.done)/\(PlanWords.total(counts))" }

    /// The plan's sections the navigator draws now: none on a board that
    /// isn't planned, so it's the task list as it always was.
    struct Shown {
        var themes: [PlanTheme] = []
        var pages: [BoardPage] = []
        /// Show Hidden Pages, under no pages at all: unfiltered only.
        var offersHidden = false

        @MainActor
        init(_ plan: PlanStore, filter: String) {
            guard plan.planned else { return }
            themes = PlanNavigator.themes(plan.plan, filter: filter)
            pages = PlanNavigator.pages(plan.listedPages, filter: filter)
            offersHidden = BoardFilter.isEmpty(filter) && plan.hiddenCount > 0
        }

        var showsPages: Bool { !pages.isEmpty || offersHidden }
        var isEmpty: Bool { themes.isEmpty && !showsPages }
    }
}

/// What tells that a lane or a theme changed (`listChanges`): what its row
/// shows. A lane's state, reason, cards, rank and whether it waits on the
/// owner; a theme's name, state, counts, next and ask.
enum PlanChanges {
    static func lanes(_ lanes: [PlanLane], _ model: PlanModel, statuses: [String: TaskStatus]) -> [ListChangeRow] {
        lanes.enumerated().map { index, lane in
            ListChangeRow(
                id: lane.id,
                signature: [
                    lane.state.rawValue, lane.reason, lane.name, model.theme(of: lane)?.name ?? "",
                    "\(lane.cards.count)", "\(lane.stale)", "\(model.waitsOnOwner(lane, statuses: statuses))",
                    lane.state == .queued ? "\(index)" : "",
                ].joined(separator: "\u{1}"))
        }
    }

    static func themes(_ themes: [PlanTheme]) -> [ListChangeRow] {
        themes.map { theme in
            ListChangeRow(
                id: theme.id,
                signature: [theme.name, theme.state, "\(theme.counts)", theme.next, theme.ownerAsk, theme.outcome]
                    .joined(separator: "\u{1}"))
        }
    }
}

/// The navigator's Themes: a row each, in the plan's order.
struct PlanNavigatorThemes: View {
    let themes: [PlanTheme]
    let selected: PlanPage?
    let keyed: Bool
    let onOpen: (PlanPage) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(themes) { theme in
                PlanThemeRow(theme: theme, selected: selected == .theme(theme.id), keyed: keyed) {
                    onOpen(.theme(theme.id))
                }
                .changeWashed(theme.id)
            }
        }
        .listChanges(PlanChanges.themes(themes))
        .identified("plan-themes")
    }
}

/// The navigator's Pages: the orchestrator's own pages, and Show Hidden
/// Pages when some are hidden here.
struct PlanNavigatorPages: View {
    @ObservedObject var plan: PlanStore
    let pages: [BoardPage]
    let selected: PlanPage?
    let keyed: Bool
    let onOpen: (PlanPage) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(pages) { page in
                PlanPageRow(
                    page: page, now: PlanOverviewView.nowMs(), selected: selected == .page(page.slot), keyed: keyed,
                    onHide: { plan.hide(page.slot) }
                ) { onOpen(.page(page.slot)) }
            }
            if plan.hiddenCount > 0 {
                Button("Show Hidden Pages") { plan.showHiddenPages() }
                    .buttonStyle(.link)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .padding(.leading, NavigatorGrid.textInset)
                    .padding(.vertical, NavigatorRhythm.air)
                    .identified("plan-show-hidden-pages")
            }
        }
        .identified("plan-pages")
    }
}

/// One theme in the navigator: its name, its progress trailing, and, in
/// amber, that it needs the owner. Its glyph is its state's, a shape to scan
/// by. The theme's page has the rest: its outcome, story, lanes and cards.
struct PlanThemeRow: View {
    let theme: PlanTheme
    let selected: Bool
    let keyed: Bool
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    private var asks: Bool { !theme.ownerAsk.isEmpty }
    private var over: PlanBudget? { PlanWords.overBudget(theme) }

    static func glyph(_ state: String) -> String {
        switch state {
        case "paused": "pause.circle"
        case "done": "checkmark.circle"
        default: "map"
        }
    }

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                Image(systemName: Self.glyph(theme.state))
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(asks || over != nil ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(SidebarInk.secondary))
                    .accessibilityHidden(true)
                    .glyphColumn()
                VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                    HStack(alignment: .firstTextBaseline, spacing: 0) {
                        Text(theme.name)
                            .font(.system(size: WorkspaceStyle.PaneText.body))
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .layoutPriority(1)
                        Spacer(minLength: SidebarGrid.gap)
                        Text(PlanNavigator.progress(theme.counts))
                            .font(.system(size: WorkspaceStyle.PaneText.secondary).monospacedDigit())
                            .foregroundStyle(SidebarInk.secondary)
                            .lineLimit(1)
                            .fixedSize()
                            .probed("plan-theme-\(theme.name)-progress")
                    }
                    // A theme past its token budget (ov-307): the one thing
                    // besides an ask that's drawn in amber, with its words.
                    if let over {
                        Text(PlanWords.budgetLine(over))
                            .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                            .foregroundStyle(Tint.attention(scheme))
                            .lineLimit(1)
                            .probed("plan-theme-\(theme.name)-over-budget")
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .navigatorRow(selected: selected, keyed: keyed, leading: 0)
            .background {
                if hovering && !selected { RoundedRectangle.control.fill(Fill.hover).boxOutset() }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(asks ? "Needs you: \(theme.ownerAsk)" : theme.outcome)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibility)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .identified("plan-theme-\(theme.name)")
    }

    private var accessibility: String {
        var parts = [theme.name, PlanWords.progress(theme.counts)]
        if theme.state != "active" { parts.append(theme.state.capitalized) }
        if asks { parts.append("Needs you: \(theme.ownerAsk)") }
        if let over { parts.append(PlanWords.budgetSpoken(over)) }
        return parts.joined(separator: ", ")
    }
}
