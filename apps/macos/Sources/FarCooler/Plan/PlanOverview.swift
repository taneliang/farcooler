import AgentKit
import SwiftUI

// The Plan view (ov-268 design 6.1): what the board's navigator shows when
// Plan is chosen. Next Up first, because it's what the owner can't see
// anywhere else; then the lanes working now, the themes as cards, and what
// landed today. A lane or a theme opens its page in the main area, where a
// task opens.

/// The board's Tasks | Plan control: shown only on a runner that keeps a
/// plan, Tasks the default.
struct PlanToggle: View {
    @ObservedObject var plan: PlanStore

    var body: some View {
        if plan.available {
            Picker("Board", selection: $plan.shown) {
                Text("Tasks").tag(false)
                Text("Plan").tag(true)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .controlSize(.small)
            .help("Show this board’s tasks, or its plan: themes, lanes and what’s next")
            .padding(.horizontal, NavigatorGrid.boxEdge)
            .padding(.top, NavigatorRhythm.band)
            .identified("board-plan-toggle")
            .focusedSceneValue(\.boardPlan, plan.shown)
        }
    }
}

/// The navigator's Tasks section with Plan chosen. A pane's content, so it
/// scrolls in the pane's own scroll view and takes the pane's inset.
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
        } else if model.isEmpty {
            PlanNotice(title: PlanWords.nothingPlanned, detail: PlanWords.nothingPlannedDetail)
                .identified("plan-empty")
        } else {
            if !model.nextUp.isEmpty { nextUp(model) }
            if !model.working.isEmpty || !model.unranked.isEmpty { now(model) }
            if !model.shownThemes.isEmpty { themes(model) }
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
                }
            }
        }
        .identified("plan-next-up")
    }

    private func now(_ model: PlanModel) -> some View {
        let lanes = model.working + model.unranked
        return CollapsibleSection("Now", id: "plan.now", style: .navigator, key: key("now"), defaults: defaults,
            count: lanes.count
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(lanes) { lane in
                    PlanLaneRow(
                        lane: lane, theme: model.theme(of: lane), rank: nil, now: model.nowMs,
                        waitsOnOwner: model.waitsOnOwner(lane, statuses: statuses),
                        selected: selected == .lane(lane.id), keyed: keyed
                    ) { onOpen(.lane(lane.id)) }
                }
            }
        }
        .identified("plan-now")
    }

    private func themes(_ model: PlanModel) -> some View {
        CollapsibleSection("Themes", id: "plan.themes", style: .navigator, key: key("themes"), defaults: defaults,
            count: model.shownThemes.count
        ) {
            // One column in a narrow navigator, more as it widens.
            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: PlanMetrics.themeCardMinimum), spacing: Spacing.group, alignment: .top)],
                alignment: .leading, spacing: Spacing.group
            ) {
                ForEach(model.shownThemes) { theme in
                    PlanThemeCard(theme: theme, selected: selected == .theme(theme.id), keyed: keyed) {
                        onOpen(.theme(theme.id))
                    }
                }
            }
            .padding(.top, NavigatorRhythm.air)
        }
        .identified("plan-themes")
    }

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
    /// A theme card's least width: two fit side by side once the navigator
    /// is about twice its usual width.
    static let themeCardMinimum: CGFloat = 220
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

    /// Stale, or waiting on the owner: the only lanes drawn in color.
    private var warning: String? {
        waitsOnOwner ? "Needs you" : PlanWords.stale(lane, now: now)
    }

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                mark.glyphColumn()
                VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                    HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                        Text(lane.name)
                            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                            .lineLimit(1)
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
                    Text(second)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                    if let warning {
                        Text(warning)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                            .foregroundStyle(Tint.attention(scheme))
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
        if rank != nil { return lane.reason.isEmpty ? PlanWords.cards(lane.cards.count) : lane.reason }
        return "\(PlanWords.status(lane)) · \(PlanWords.cards(lane.cards.count))"
    }

    @ViewBuilder private var mark: some View {
        if let rank {
            Text("\(rank)")
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold).monospacedDigit())
                .foregroundStyle(.secondary)
        } else {
            Image(systemName: PlanGlyph.name(lane.state))
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(warning == nil ? AnyShapeStyle(.secondary) : AnyShapeStyle(Tint.attention(scheme)))
                .accessibilityHidden(true)
        }
    }

    private var accessibility: String {
        var parts = [rank.map { "\(PlanWords.ordinal($0)) up" }, lane.name, theme?.name, second, warning]
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

/// A theme's card in the overview: its name and progress, its outcome, the
/// bar, what's next and, in amber, what needs the owner.
struct PlanThemeCard: View {
    let theme: PlanTheme
    let selected: Bool
    let keyed: Bool
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: Spacing.tight) {
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    Text(theme.name)
                        .font(.system(size: WorkspaceStyle.PaneText.title, weight: .semibold))
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    if theme.state != "active" {
                        Text(theme.state.capitalized)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(.secondary)
                    }
                }
                if !theme.outcome.isEmpty {
                    Text(theme.outcome)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                PlanBar(counts: theme.counts)
                    .padding(.top, Spacing.tight)
                Text(PlanWords.progress(theme.counts))
                    .font(.system(size: WorkspaceStyle.PaneText.secondary).monospacedDigit())
                    .foregroundStyle(.secondary)
                if !theme.next.isEmpty {
                    (Text("Next: ").foregroundStyle(.secondary) + Text(theme.next))
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, Spacing.tight)
                }
                if !theme.ownerAsk.isEmpty {
                    PlanAsk(text: theme.ownerAsk, lines: 2)
                }
            }
            .padding(Spacing.inset)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .surface(.inset, in: .card)
            .background {
                if selected {
                    RoundedRectangle.card.fill(Fill.selection(active: keyed))
                } else if hovering {
                    RoundedRectangle.card.fill(Fill.hover)
                }
            }
            .contentShape(.card)
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .identified("plan-theme-\(theme.name)")
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

/// The board's tasks, or with Plan chosen its plan: the one switch between
/// them, so the tasks are drawn exactly as before whenever Plan isn't.
struct PlanOrTasks<Tasks: View, Overview: View>: View {
    @ObservedObject var plan: PlanStore
    @ViewBuilder let tasks: () -> Tasks
    @ViewBuilder let overview: () -> Overview

    var body: some View {
        if plan.showing {
            overview()
        } else {
            tasks()
        }
    }
}

extension TaskBoardModel {
    /// Each task's status by id: what the Plan view asks whether a lane
    /// waits on the owner.
    var statuses: [String: TaskStatus] {
        Dictionary(rows.map { ($0.id, $0.status) }, uniquingKeysWith: { a, _ in a })
    }
}
