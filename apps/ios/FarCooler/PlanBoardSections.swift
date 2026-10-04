import SwiftUI

// The Plan view on a phone (ov-268 design 6.5): what the board's list shows
// when Plan is chosen. Next Up first, because it's what the owner can't see
// anywhere else; then the lanes working now, the themes, and what landed
// today. A lane or a theme opens its page, pushed.
//
// EXPERIMENTAL and opt-in, as the layer is: Tasks is the default, the control
// only appears on a runner that advertises `board_plan`, and it switches only
// the task list's sections. The Unread strip, the waiting count, the pull to
// refresh and everything above and below stay where they are.
//
// Every rule is AgentKit's (`PlanModel`, `PlanWords`, `PlanReadState`); this
// draws, in iOS's own idiom: title case, system type, and color only for a
// lane that's stale or waiting on you.

/// What `WorkspaceBoardList` needs to draw a board's plan.
struct PlanBoardHook {
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    let reads: PlanReads
    /// Whether this runner advertises `board_plan`.
    let keeps: Bool
    /// Read the plan, once or again.
    let read: () async -> Void
    let onOpen: (PhonePlanPage) -> Void
    /// The board's statuses by task id: a lane waits on you when one of its
    /// cards needs a decision.
    let statuses: [String: TaskStatus]
}

/// The Tasks | Plan control, a segmented picker as Apple's own lists have it.
struct PlanSwitch: View {
    @Binding var showsPlan: Bool

    var body: some View {
        Picker("Board", selection: $showsPlan) {
            Text("Tasks").tag(false)
            Text("Plan").tag(true)
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .accessibilityIdentifier("plan-switch")
    }
}

/// The Plan view's sections, in a board's list.
struct PlanBoardSections: View {
    let hook: PlanBoardHook
    @ObservedObject var reads: PlanReads
    @State private var landedOpen = false

    init(hook: PlanBoardHook) {
        self.hook = hook
        reads = hook.reads
    }

    var body: some View {
        switch reads.state(hook.summary.id) {
        case .none, .loading:
            Section {
                ProgressView()
                    .frame(maxWidth: .infinity, alignment: .center)
                    .accessibilityLabel("Reading the plan")
                    .accessibilityIdentifier("plan-loading")
            }
        case .needsUpdate:
            Section { PlanNotice(title: PlanWords.needsUpdate, detail: nil).accessibilityIdentifier("plan-needs-update") }
        case .unavailable:
            Section {
                PlanNotice(title: PlanWords.couldntRead, detail: nil)
                    .accessibilityIdentifier("plan-unavailable")
                Button(PlanWords.tryAgain) { Task { await hook.read() } }
                    .accessibilityIdentifier("plan-retry")
            }
        case .loaded(let plan):
            sections(plan)
        }
    }

    @ViewBuilder
    private func sections(_ plan: PlanModel) -> some View {
        if plan.isEmpty {
            Section {
                PlanNotice(title: PlanWords.nothingPlanned, detail: PlanWords.nothingPlannedDetail)
                    .accessibilityIdentifier("plan-empty")
            }
        } else {
            if !plan.nextUp.isEmpty {
                Section {
                    ForEach(Array(plan.nextUp.enumerated()), id: \.element.id) { index, lane in
                        PlanLaneRow(
                            lane: lane, theme: plan.theme(of: lane), rank: index + 1, now: plan.nowMs,
                            waitsOnOwner: false
                        ) { hook.onOpen(.lane(lane.id)) }
                    }
                } header: {
                    PlanHeader(title: "Next Up", count: plan.nextUp.count).accessibilityIdentifier("plan-next-up")
                }
            }
            let now = plan.working + plan.unranked
            if !now.isEmpty {
                Section {
                    ForEach(now) { lane in
                        PlanLaneRow(
                            lane: lane, theme: plan.theme(of: lane), rank: nil, now: plan.nowMs,
                            waitsOnOwner: plan.waitsOnOwner(lane, statuses: hook.statuses)
                        ) { hook.onOpen(.lane(lane.id)) }
                    }
                } header: {
                    PlanHeader(title: "Now", count: now.count).accessibilityIdentifier("plan-now")
                }
            }
            if !plan.shownThemes.isEmpty {
                Section {
                    ForEach(plan.shownThemes) { theme in
                        PlanThemeRow(theme: theme) { hook.onOpen(.theme(theme.id)) }
                    }
                } header: {
                    PlanHeader(title: "Themes", count: plan.shownThemes.count).accessibilityIdentifier("plan-themes")
                }
            }
            let landed = plan.landedToday()
            if !landed.isEmpty {
                Section {
                    if landedOpen {
                        ForEach(landed) { lane in
                            PlanLaneRow(
                                lane: lane, theme: plan.theme(of: lane), rank: nil, now: plan.nowMs,
                                waitsOnOwner: false
                            ) { hook.onOpen(.lane(lane.id)) }
                        }
                    }
                } header: {
                    Button { withAnimation { landedOpen.toggle() } } label: {
                        PlanHeader(title: "Landed Today", count: landed.count, open: landedOpen)
                    }
                    .buttonStyle(.plain)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("Landed Today, \(landed.count)")
                    .accessibilityValue(landedOpen ? "Expanded" : "Collapsed")
                    .accessibilityIdentifier("plan-landed-header")
                }
            }
        }
    }
}

/// A section's title and count, as the board's status headers read: the name
/// and its count, trailing and tertiary, with a chevron when it opens.
struct PlanHeader: View {
    let title: String
    let count: Int
    /// Nil when the section doesn't open.
    var open: Bool?

    var body: some View {
        HStack(spacing: PaneMetrics.tight) {
            Text(title)
            Spacer()
            Text("\(count)")
                .font(.subheadline.monospacedDigit())
                .foregroundStyle(.tertiary)
            if let open {
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .rotationEffect(.degrees(open ? 90 : 0))
                    .foregroundStyle(.tertiary)
            }
        }
        .textCase(nil)
        .font(.subheadline.weight(.semibold))
        .foregroundStyle(.secondary)
    }
}

/// "Nothing is planned on this board yet.", and why there would be.
struct PlanNotice: View {
    let title: String
    let detail: String?

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.tight) {
            Text(title).font(.subheadline.weight(.semibold))
            if let detail {
                Text(detail).font(.footnote).foregroundStyle(.secondary)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, alignment: .leading)
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

/// One lane's row: its name and the theme it serves, then its reason or
/// state. In Next Up its rank sits in the glyph column; elsewhere its state's
/// glyph does, amber only when it's stale or waiting on you.
struct PlanLaneRow: View {
    let lane: PlanLane
    let theme: PlanTheme?
    /// Its place in Next Up, or nil.
    let rank: Int?
    let now: Int64
    let waitsOnOwner: Bool
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme

    /// Stale, or waiting on you: the only lanes drawn in color.
    private var warning: String? {
        waitsOnOwner ? "Needs you" : PlanWords.stale(lane, now: now)
    }

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.card) {
                mark.frame(width: 22, alignment: .leading)
                VStack(alignment: .leading, spacing: 2) {
                    // Which theme it serves (the owner's ask, ov-273): beside
                    // the name, or under it when the name takes the line.
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.step) {
                            name
                            Spacer(minLength: 0)
                            themeName
                        }
                        VStack(alignment: .leading, spacing: 2) {
                            name
                            themeName
                        }
                    }
                    Text(second)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                    if let warning {
                        Text(warning)
                            .font(.footnote.weight(.medium))
                            .foregroundStyle(GlancePalette.amber(scheme))
                    }
                }
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibility)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("plan-lane-\(lane.name)")
    }

    private var name: some View {
        Text(lane.name)
            .font(.subheadline.weight(.medium))
            .lineLimit(1)
            .layoutPriority(1)
    }

    @ViewBuilder private var themeName: some View {
        if let theme {
            Text(theme.name)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .accessibilityIdentifier("plan-lane-\(lane.name)-theme")
        }
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
                .font(.subheadline.monospacedDigit())
                .foregroundStyle(.secondary)
        } else {
            Image(systemName: PlanGlyph.name(lane.state))
                .font(.subheadline)
                .foregroundStyle(warning == nil ? AnyShapeStyle(.secondary) : AnyShapeStyle(GlancePalette.amber(scheme)))
                .accessibilityHidden(true)
        }
    }

    private var accessibility: String {
        var parts = [rank.map { "\(PlanWords.ordinal($0)) up" }, lane.name, theme?.name, second, warning]
        parts.removeAll { $0 == nil }
        return parts.compactMap { $0 }.joined(separator: ", ")
    }
}

/// A theme in the overview: its name and outcome, its progress, what's next
/// and, in amber, what needs you. Each theme reads on its own (the owner's
/// goal, ov-273).
struct PlanThemeRow: View {
    let theme: PlanTheme
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: action) {
            HStack(alignment: .top, spacing: PaneMetrics.step) {
                VStack(alignment: .leading, spacing: PaneMetrics.tight) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(theme.name).font(.headline).lineLimit(2)
                        if theme.state != "active" {
                            Text(theme.state.capitalized).font(.footnote).foregroundStyle(.secondary)
                        }
                    }
                    if !theme.outcome.isEmpty {
                        Text(theme.outcome)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .lineLimit(PlanWords.outcomeLines)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    PlanProgressBar(counts: theme.counts).padding(.top, PaneMetrics.tight)
                    Text(PlanWords.progress(theme.counts))
                        .font(.footnote.monospacedDigit())
                        .foregroundStyle(.secondary)
                    if !theme.next.isEmpty {
                        Text("\(Text("Next: ").foregroundStyle(.secondary))\(theme.next)")
                            .font(.footnote)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if !theme.ownerAsk.isEmpty { PlanAsk(text: theme.ownerAsk, lines: 2) }
                }
                Spacer(minLength: 0)
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .padding(.top, 4)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, 2)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("plan-theme-\(theme.name)")
    }
}

/// "● Needs you: …": the one colored mark a theme has, with words.
struct PlanAsk: View {
    let text: String
    var lines: Int?
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Circle()
                .fill(GlancePalette.amber(scheme))
                .frame(width: 6, height: 6)
                .accessibilityHidden(true)
            Text("\(Text("Needs you: ").fontWeight(.medium).foregroundStyle(GlancePalette.amber(scheme)))\(text)")
                .font(.footnote)
                .lineLimit(lines)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.top, PaneMetrics.tight)
        .accessibilityIdentifier("plan-ask")
    }
}

/// A theme's cards by status, left to right, in neutral fills: done, in
/// review, in progress, not started. Canceled cards aren't drawn
/// (`PlanWords.segments`).
struct PlanProgressBar: View {
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
        .frame(height: 4)
        .background(Capsule().fill(.quinary))
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
