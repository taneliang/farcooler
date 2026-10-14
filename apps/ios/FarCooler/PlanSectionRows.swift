import SwiftUI

// The Plan's sections on a phone and in the iPad's plan column (ov-503,
// design 2 and 7; ov-522): Landing, In Review, Building, Waiting, On a Landed Train
// (folded), Agents, then Themes as a grid and, on a phone, Themes and Lanes
// as two rows that push lists. `PlanSections` decides what is where and what
// each says empty; this lays it out as an inset list's sections, one column.

struct PlanSectionsList: View {
    let sections: PlanSections
    let plan: PlanModel
    let hook: PlanBoardHook

    @State private var landedOpen = !PlanSections.Kind.onLandedTrain.startsFolded
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        // In `shown`'s order, so what is drawn is what the model says, in the
        // order it says (owner decision 7; ov-522).
        ForEach(sections.shown, id: \.self) { kind in
            switch kind {
            case .needsYou: EmptyView()
            case .landing: landing
            case .inReview: laneSection(.inReview, id: "plan-in-review", rows: sections.inReview)
            case .building: laneSection(.building, id: "plan-building", rows: sections.building)
            case .waiting: waiting
            case .onLandedTrain: landedTrain
            case .agents: agents
            case .themes: themes
            }
        }
        if hook.pushesLists, hook.outline != nil { browse }
    }

    // MARK: Rows

    private func laneRow(_ row: PlanSections.LaneRow, rank: Int? = nil) -> some View {
        PlanLaneRow(
            lane: row.lane, theme: plan.theme(of: row.lane), rank: rank, now: sections.now, waitsOnOwner: row.waitsOnOwner,
            agent: row.agent, landedTrain: row.landedTrain
        ) { hook.onOpen(.lane(row.id)) }
    }

    /// What a section says with nothing in it.
    private func empty(_ words: String, id: String) -> some View {
        Text(words)
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .accessibilityIdentifier("\(id)-empty")
    }

    // MARK: Landing

    private var landing: some View {
        Section {
            if let words = sections.emptyWords(.landing) {
                empty(words, id: "plan-landing")
            } else {
                ForEach(sections.trains) { card in
                    PlanTrainRow(card: card, ci: plan.ci(of: card.train), now: sections.now)
                    ForEach(card.lanes) { lane in
                        PlanLaneRow(
                            lane: lane.lane, theme: plan.theme(of: lane.lane), rank: nil, now: sections.now,
                            waitsOnOwner: lane.waitsOnOwner, standing: lane.waitsOnOwner ? "" : lane.note ?? ""
                        ) { hook.onOpen(.lane(lane.id)) }
                        .padding(.leading, PaneMetrics.card)
                    }
                }
            }
        } header: {
            PlanHeader(title: PlanSections.Kind.landing.title, count: sections.count(.landing) == 0 ? nil : sections.count(.landing))
                .explainedOnLongPress([PlanWords.trainExplanation])
                .accessibilityIdentifier("plan-landing")
        }
    }

    // MARK: In Review and Building

    private func laneSection(_ kind: PlanSections.Kind, id: String, rows: [PlanSections.LaneRow]) -> some View {
        Section {
            if let words = sections.emptyWords(kind) {
                empty(words, id: id)
            } else {
                ForEach(rows) { laneRow($0) }
            }
        } header: {
            PlanHeader(title: kind.title, count: rows.isEmpty ? nil : rows.count).accessibilityIdentifier(id)
        }
    }

    // MARK: Waiting

    private var waiting: some View {
        let waiting = sections.waiting
        let groups = waiting.groups
        return Section {
            if let words = sections.emptyWords(.waiting) {
                empty(words, id: "plan-waiting")
            } else {
                ForEach(groups) { group in
                    // Each group is named only when something else shares the section.
                    if waiting.namesGroups { groupTitle(group) }
                    ForEach(Array(group.rows.enumerated()), id: \.element.id) { index, row in
                        laneRow(row, rank: group.rank(at: index))
                    }
                }
            }
        } header: {
            PlanHeader(title: PlanSections.Kind.waiting.title, count: waiting.isEmpty ? nil : waiting.count)
                .accessibilityIdentifier("plan-waiting")
        }
    }

    private func groupTitle(_ group: PlanSections.Waiting.Group) -> some View {
        HStack(spacing: 6) {
            if group.attention {
                Circle().fill(GlancePalette.amber(scheme)).frame(width: 6, height: 6).accessibilityHidden(true)
            }
            Text(group.title).font(.footnote.weight(.semibold)).foregroundStyle(.secondary)
        }
        .listRowSeparator(.hidden, edges: .bottom)
        // A label, not a row: its words sit close over the rows they name
        // rather than floating in a 44 pt one (ov-515). The list's minimum row
        // height is lowered where the Plan draws it (`PlanRowHeight`).
        .frame(maxWidth: .infinity, alignment: .leading)
        .listRowInsets(EdgeInsets(top: 10, leading: 16, bottom: 2, trailing: 16))
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
        .accessibilityIdentifier(group.id)
    }

    // MARK: On a Landed Train

    private var landedTrain: some View {
        Section {
            if landedOpen { ForEach(sections.onLandedTrain) { laneRow($0) } }
        } header: {
            PlanHeader(title: PlanSections.Kind.onLandedTrain.title, count: sections.onLandedTrain.count, open: landedOpen)
                .contentShape(.rect)
                .onTapGesture { withAnimation { landedOpen.toggle() } }
                .accessibilityElement(children: .ignore)
            .accessibilityLabel("\(PlanSections.Kind.onLandedTrain.title), \(sections.onLandedTrain.count)")
            .accessibilityValue(landedOpen ? "Expanded" : "Collapsed")
            .accessibilityAddTraits(.isButton)
            .accessibilityIdentifier("plan-on-landed-train")
        }
    }

    // MARK: Agents

    private var agents: some View {
        let fleet = sections.fleet
        return Section {
            if fleet.isEmpty {
                empty(fleet.header(), id: "plan-agents")
            } else {
                Text(fleet.header())
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("plan-agents-header")
                ForEach(fleet.agents) { agent in
                    PlanAgentRow(agent: agent, now: sections.now) { open($0) }
                }
            }
        } header: {
            PlanHeader(title: PlanSections.Kind.agents.title, count: nil).accessibilityIdentifier("plan-agents")
        }
    }

    private func open(_ lane: PlanFleetAgent.Lane) { hook.onOpen(.lane(lane.id)) }

    // MARK: Themes

    private var themes: some View {
        Section {
            PlanThemeGridView(grid: sections.themes, plan: plan, columns: hook.tileColumns) { hook.onOpen(.theme($0)) }
                .listRowInsets(EdgeInsets())
                .listRowBackground(Color.clear)
                .listRowSeparator(.hidden)
        } header: {
            PlanHeader(title: PlanSections.Kind.themes.title, count: sections.count(.themes)).accessibilityIdentifier("plan-themes")
        } footer: {
            if let summary = plan.trackSummary() {
                Text(summary).accessibilityIdentifier("plan-themes-summary")
            }
        }
    }

    // MARK: Themes and Lanes, pushed

    private var browse: some View {
        Section {
            NavigationLink {
                PlanBrowseList(hook: hook, kind: .themes)
            } label: {
                Label("Themes", systemImage: "square.grid.2x2").foregroundStyle(.primary)
            }
            .accessibilityIdentifier("plan-browse-themes")
            NavigationLink {
                PlanBrowseList(hook: hook, kind: .lanes)
            } label: {
                Label("Lanes", systemImage: "point.topleft.down.to.point.bottomright.curvepath").foregroundStyle(.primary)
            }
            .accessibilityIdentifier("plan-browse-lanes")
        }
    }
}

/// One agent that exists, working or idle: who (role and model), what for and
/// its state, and its tokens. A working agent has a filled dot, an idle one a
/// ring; a tap opens its lane.
struct PlanAgentRow: View {
    let agent: PlanFleetAgent
    let now: Int64
    let onOpen: (PlanFleetAgent.Lane) -> Void
    @ScaledMetric(relativeTo: .subheadline) private var markWidth: CGFloat = 22

    var body: some View {
        let row = content
        Group {
            if let lane = agent.lane {
                Button { onOpen(lane) } label: { row }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(.isButton)
            } else {
                row
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(agent.spoken(now: now))
        .accessibilityIdentifier("plan-agent-\(agent.id)")
    }

    private var content: some View {
        HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.card) {
            Image(systemName: agent.working ? "circle.fill" : "circle")
                .font(.system(size: 7))
                .foregroundStyle(agent.working ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
                .frame(width: markWidth, alignment: .leading)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(agent.who)
                    .font(.subheadline.weight(.medium))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Text(agent.second(now: now))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 4)
            if let tokens = agent.tokens() {
                Text(tokens)
                    .font(.footnote.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .fixedSize()
            }
            if agent.lane != nil {
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
        }
        .contentShape(.rect)
    }
}
