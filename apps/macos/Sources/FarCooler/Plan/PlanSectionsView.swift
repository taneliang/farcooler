import AgentKit
import SwiftUI

// The Plan's sections, drawn (ov-503, design 2). `PlanSections` decides what is
// where and what each says empty; this lays it out. Wide, the work in motion
// (Landing, In Review, Building) fills the left column and what is queued and
// what it costs (Waiting, On a Landed Train, Agents) the right, the themes
// spanning both below; narrow, the same sections stack in the same order.

/// Two columns when there's room for two, else one: the first subview left, the
/// second right, each as wide as the other.
struct PlanColumnsLayout: Layout {
    var threshold: CGFloat = PlanMetrics.twoColumns
    var gap: CGFloat = PlanMetrics.columnGap
    var spacing: CGFloat = NavigatorRhythm.section

    private func sides(_ width: CGFloat?) -> Bool { (width ?? 0) >= threshold && width != nil }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        if sides(proposal.width), let width = proposal.width {
            let each = ProposedViewSize(width: (width - gap) / 2, height: nil)
            return CGSize(width: width, height: subviews.map { $0.sizeThatFits(each).height }.max() ?? 0)
        }
        let sizes = subviews.map { $0.sizeThatFits(ProposedViewSize(width: proposal.width, height: nil)) }
        let drawn = sizes.filter { $0.height > 0 }
        return CGSize(
            width: sizes.map(\.width).max() ?? 0,
            height: drawn.map(\.height).reduce(0, +) + spacing * CGFloat(max(0, drawn.count - 1)))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        if sides(bounds.width) {
            let each = (bounds.width - gap) / 2
            for (index, view) in subviews.enumerated() {
                view.place(
                    at: CGPoint(x: bounds.minX + CGFloat(index) * (each + gap), y: bounds.minY), anchor: .topLeading,
                    proposal: ProposedViewSize(width: each, height: nil))
            }
            return
        }
        var y = bounds.minY
        for view in subviews {
            let size = view.sizeThatFits(ProposedViewSize(width: bounds.width, height: nil))
            view.place(at: CGPoint(x: bounds.minX, y: y), anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width, height: nil))
            if size.height > 0 { y += size.height + spacing }
        }
    }
}

/// One of Waiting's groups.
private struct WaitingGroup {
    let title: String
    let rows: [PlanSections.LaneRow]
    let ranked: Bool
    let tone: SectionTitle.Tone
    let id: String
}

struct PlanSectionsView: View {
    let sections: PlanSections
    let model: PlanModel
    let statuses: [String: TaskStatus]
    @ObservedObject var plan: PlanStore
    let selected: PlanPage?
    let keyed: Bool
    let defaults: UserDefaults
    let onOpen: (PlanPage) -> Void
    /// Lanes being taken off a train now.
    @State private var moving: Set<String> = []

    /// The left column's sections: the work in motion.
    private static let left: [PlanSections.Kind] = [.landing, .inReview, .building]

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.section) {
            PlanColumnsLayout {
                column(Self.left)
                column([.waiting, .onLandedTrain, .agents])
            }
            if sections.shown.contains(.themes) { themes }
        }
        .identified("plan-sections")
    }

    private func column(_ kinds: [PlanSections.Kind]) -> some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.section) {
            ForEach(sections.shown.filter { kinds.contains($0) }, id: \.self) { section($0) }
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }

    private func key(_ section: String) -> String { "board.plan.section.\(section).\(plan.host).\(plan.workspace.id)" }

    @ViewBuilder private func section(_ kind: PlanSections.Kind) -> some View {
        switch kind {
        case .landing: landing
        case .inReview: laneSection(kind, id: "plan-in-review", rows: sections.inReview)
        case .building: laneSection(kind, id: "plan-building", rows: sections.building)
        case .waiting: waiting
        case .onLandedTrain: landedTrain
        case .agents: agents
        case .needsYou, .themes: EmptyView()
        }
    }

    /// A section's rows, or what it says empty.
    /// `help` is said on hover over the section's title only, never its rows.
    private func chrome<Content: View>(
        _ kind: PlanSections.Kind, id: String, expanded: Bool = true, help: String? = nil,
        @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        CollapsibleSection(
            id: "plan.\(id)", style: .navigator, key: key(id), defaults: defaults, expandedByDefault: expanded,
            count: sections.count(kind) == 0 ? nil : sections.count(kind), accessibilityLabel: kind.title,
            label: { _ in
                SectionTitle(
                    text: kind.title, style: .navigator, tone: .primary,
                    gridRow: CollapsibleSection<SectionTitle, EmptyView, EmptyView>.gridRow("plan.\(id)")
                )
                .help(help ?? "")
            },
            accessory: { EmptyView() }
        ) {
            if let empty = sections.emptyWords(kind) {
                PlanEmptyLine(words: empty).identified("\(id)-empty")
                // Where the queue's work comes from, said where it's empty.
                if kind == .waiting { PlanNextStep() }
            } else {
                content()
            }
        }
        .identified(id)
    }

    private var landing: some View {
        chrome(.landing, id: "plan-landing", help: PlanWords.trainExplanation) {
            VStack(alignment: .leading, spacing: Spacing.group) {
                ForEach(sections.trains) { card in
                    PlanTrainCard(
                        card: card, ci: model.ci(of: card.train), now: sections.now, canMove: plan.movesLanes, moving: moving,
                        selected: selected, keyed: keyed, onOpen: onOpen, onMove: take
                    )
                    .changeWashed(card.id)
                }
            }
            .padding(.top, NavigatorRhythm.air)
            .listChanges(
                sections.trains.map {
                    ListChangeRow(
                        id: $0.id,
                        signature: [
                            $0.phase.rawValue, $0.held ?? "", PlanTrainCard.facts($0, ci: model.ci(of: $0.train), now: 0),
                            $0.lanes.map { "\($0.id)\($0.standing.word)" }.joined(separator: ","),
                        ].joined(separator: "\u{1}"))
                })
        }
    }

    private func laneSection(_ kind: PlanSections.Kind, id: String, rows: [PlanSections.LaneRow]) -> some View {
        chrome(kind, id: id) { laneRows(rows) }
    }

    private func laneRows(_ rows: [PlanSections.LaneRow], rank: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                PlanLaneRow(
                    lane: row.lane, theme: model.theme(of: row.lane), rank: rank ? index + 1 : nil, now: sections.now,
                    waitsOnOwner: row.waitsOnOwner, selected: selected == .lane(row.id), keyed: keyed, agent: row.agent,
                    landedTrain: row.landedTrain
                ) { onOpen(.lane(row.id)) }
                .changeWashed(row.id)
            }
        }
        .listChanges(PlanChanges.lanes(rows.map(\.lane), model, statuses: statuses))
    }

    /// Next Up ranked, then Not Ranked, then Waiting on You, each named only
    /// when something else shares the section.
    private var waiting: some View {
        chrome(.waiting, id: "plan-waiting") {
            let waiting = sections.waiting
            let groups = [
                WaitingGroup(title: "Next Up", rows: waiting.next, ranked: true, tone: .primary, id: "plan-next-up"),
                WaitingGroup(title: "Not Ranked", rows: waiting.notRanked, ranked: false, tone: .primary, id: "plan-not-ranked"),
                WaitingGroup(title: "Waiting on You", rows: waiting.onYou, ranked: false, tone: .attention, id: "plan-waiting-on-you"),
            ].filter { !$0.rows.isEmpty }
            VStack(alignment: .leading, spacing: NavigatorRhythm.group) {
                ForEach(groups, id: \.id) { group in
                    VStack(alignment: .leading, spacing: 0) {
                        if groups.count > 1 {
                            SectionTitle(text: group.title, style: .minor, tone: group.tone)
                                .padding(.leading, NavigatorGrid.textInset)
                                .padding(.vertical, NavigatorRhythm.air)
                        }
                        laneRows(group.rows, rank: group.ranked)
                    }
                    .identified(group.id)
                }
            }
        }
    }

    private var landedTrain: some View {
        chrome(.onLandedTrain, id: "plan-on-landed-train", expanded: false) { laneRows(sections.onLandedTrain) }
    }

    private var agents: some View {
        chrome(.agents, id: "plan-agents") {
            PlanAgentsView(fleet: sections.fleet, now: sections.now, onOpen: onOpen)
        }
    }

    /// Themes: the grid, with the work outside any theme as a footer.
    private var themes: some View {
        let count = sections.themes.tiles.count + sections.themes.finished.count
        return CollapsibleSection(
            "Themes", id: "plan.themes", style: .navigator, key: key("themes"), defaults: defaults, count: count
        ) {
            VStack(alignment: .leading, spacing: Spacing.group) {
                PlanThemeGridView(grid: sections.themes, model: model, selected: selected, keyed: keyed, onOpen: onOpen)
                PlanOutsideRow(outside: model.outsideThemes(statuses: statuses), onOpen: onOpen)
            }
            .padding(.top, NavigatorRhythm.air)
        }
        .identified("plan-themes")
    }

    /// Take `lane` off its train, its button waiting while the runner answers.
    private func take(_ lane: PlanLane) {
        guard !moving.contains(lane.id) else { return }
        moving.insert(lane.id)
        Task {
            await plan.move(lane)
            moving.remove(lane.id)
        }
    }
}
