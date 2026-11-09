import AgentKit
import SwiftUI

// The Plan's sections, drawn (ov-503, design 2). The Rust core's view
// (`PlanView`, ov-1016) decides what is where, every word, and what each says
// empty; this lays it out. Wide, the work in motion
// (Landing, In Review, Building) fills the left column and what is queued and
// what it costs (Waiting, Waiting for the Next Landing, Agents) the right, the themes
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

struct PlanSectionsView: View {
    let view: PlanView
    let model: PlanModel
    let statuses: [String: TaskStatus]
    @ObservedObject var plan: PlanStore
    let selected: PlanPage?
    let keyed: Bool
    let defaults: UserDefaults
    let onOpen: (PlanPage) -> Void
    /// The left column's sections: the work in motion.
    private static let left: [PlanView.SectionKind] = [.landing, .inReview, .building]
    private static let right: [PlanView.SectionKind] = [.waiting, .onLandedLanding, .agents]

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.section) {
            PlanColumnsLayout {
                column(Self.left)
                column(Self.right)
            }
            if let themes = view.section(.themes) { self.themes(themes) }
        }
        .identified("plan-sections")
    }

    private func column(_ kinds: [PlanView.SectionKind]) -> some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.section) {
            ForEach(view.sections.filter { kinds.contains($0.kind) }) { section($0) }
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }

    private func key(_ section: String) -> String { "board.plan.section.\(section).\(plan.host).\(plan.workspace.id)" }

    @ViewBuilder private func section(_ section: PlanView.Section) -> some View {
        switch section.kind {
        case .landing: landing(section)
        case .inReview: chrome(section, id: "plan-in-review") { cardRows(view.inReview) }
        case .building: chrome(section, id: "plan-building") { cardRows(view.building) }
        case .waiting: waiting(section)
        case .onLandedLanding: chrome(section, id: "plan-on-landed-landing") { cardRows(view.onLandedLanding) }
        case .agents: agents(section)
        case .needsYou, .themes, .unknown: EmptyView()
        }
    }

    /// A section's rows, or what it says empty. Its help is said on hover
    /// over its title only, never its rows; a folded one starts closed.
    private func chrome<Content: View>(
        _ section: PlanView.Section, id: String, @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        CollapsibleSection(
            id: "plan.\(id)", style: .navigator, key: key(id), defaults: defaults, expandedByDefault: !section.folded,
            count: section.count == 0 ? nil : section.count, accessibilityLabel: section.title,
            label: { _ in
                SectionTitle(
                    text: section.title, style: .navigator, tone: .primary,
                    gridRow: CollapsibleSection<SectionTitle, EmptyView, EmptyView>.gridRow("plan.\(id)")
                )
                .help(section.help ?? "")
            },
            accessory: { EmptyView() }
        ) {
            if let empty = section.empty {
                PlanEmptyLine(words: empty).identified("\(id)-empty")
                // Where the queue's work comes from, said where it's empty.
                if section.kind == .waiting { PlanNextStep() }
            } else {
                content()
            }
        }
        .identified(id)
    }

    private func landing(_ section: PlanView.Section) -> some View {
        chrome(section, id: "plan-landing") {
            VStack(alignment: .leading, spacing: Spacing.group) {
                ForEach(view.landings) { card in
                    PlanLandingCard(
                        card: card, explanation: section.help, ci: ci(card), now: view.now,
                        selected: selected, keyed: keyed, onOpen: onOpen
                    )
                    .changeWashed(card.id)
                }
            }
            .padding(.top, NavigatorRhythm.air)
            .listChanges(
                view.landings.map {
                    ListChangeRow(
                        id: $0.id,
                        signature: [
                            $0.phase, PlanLandingCard.facts($0, ci: ci($0), now: 0),
                            $0.tickets.map { "\($0.id)\($0.standing)" }.joined(separator: ","),
                        ].joined(separator: "\u{1}"))
                })
        }
    }

    /// The CI read of a landing's pushed commit, the app's own.
    private func ci(_ card: PlanView.LandingCard) -> PlanCIRead? {
        model.landings.first { $0.id == card.id }.flatMap { model.ci(of: $0) }
    }

    private func cardRows(_ cards: [PlanView.TicketCard]) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(cards) { card in
                PlanCardRow(card: card, selected: selected == .ticket(card.id), keyed: keyed) { onOpen(.ticket(card.id)) }
                    .changeWashed(card.id)
            }
        }
        .listChanges(PlanChanges.tickets(lines(cards), model, statuses: statuses))
    }

    /// The plan's lines of `cards`, in their order, for what changed since the last read.
    private func lines(_ cards: [PlanView.TicketCard]) -> [PlanTicketLine] {
        let byTask = Dictionary(model.lines.map { ($0.task, $0) }, uniquingKeysWith: { first, _ in first })
        return cards.compactMap { byTask[$0.id] }
    }

    /// Next Up ranked, then Not Ranked, then Waiting on You, each named only
    /// when something else shares the section.
    private func waiting(_ section: PlanView.Section) -> some View {
        chrome(section, id: "plan-waiting") {
            VStack(alignment: .leading, spacing: NavigatorRhythm.group) {
                ForEach(view.waiting) { group in
                    VStack(alignment: .leading, spacing: 0) {
                        if view.namesWaitingGroups {
                            SectionTitle(text: group.title, style: .minor, tone: group.attention ? .attention : .primary)
                                .padding(.leading, NavigatorGrid.textInset)
                                .padding(.vertical, NavigatorRhythm.air)
                        }
                        cardRows(group.cards)
                    }
                    .identified(group.id)
                }
            }
        }
    }

    private func agents(_ section: PlanView.Section) -> some View {
        chrome(section, id: "plan-agents") {
            PlanAgentsView(
                agents: view.agents,
                opens: Dictionary(model.agents.compactMap { a in a.tickets.first.map { (a.id, $0.task) } }, uniquingKeysWith: { first, _ in first }),
                onOpen: onOpen)
        }
    }

    /// Themes: the grid, with the work outside any theme as a footer.
    private func themes(_ section: PlanView.Section) -> some View {
        CollapsibleSection(
            section.title, id: "plan.themes", style: .navigator, key: key("themes"), defaults: defaults, count: section.count
        ) {
            VStack(alignment: .leading, spacing: Spacing.group) {
                PlanThemeGridView(grid: view.themes, model: model, selected: selected, keyed: keyed, onOpen: onOpen)
                PlanOutsideRow(outside: model.outsideThemes(statuses: statuses), onOpen: onOpen)
            }
            .padding(.top, NavigatorRhythm.air)
        }
        .identified("plan-themes")
    }
}
