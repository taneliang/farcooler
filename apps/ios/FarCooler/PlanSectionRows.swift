import SwiftUI

// The Plan's sections on a phone and in the iPad's plan column (ov-503,
// design 2 and 7; ov-522): Landing, In Review, Building, Waiting, Waiting for the Next Landing
// (folded), Agents, then Themes as a grid and, on a phone, Themes and Tickets
// as two rows that push lists. `PlanSections` decides what is where and what
// each says empty; this lays it out as an inset list's sections, one column.

struct PlanSectionsList: View {
    let sections: PlanSections
    let plan: PlanModel
    let hook: PlanBoardHook

    @State private var landedOpen = !PlanSections.Kind.onLandedLanding.startsFolded
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        // In `shown`'s order, so what is drawn is what the model says, in the
        // order it says (owner decision 7; ov-522).
        ForEach(sections.shown.filter { hook.kinds?.contains($0) ?? true }, id: \.self) { kind in
            switch kind {
            case .needsYou: EmptyView()
            case .landing: landing
            case .inReview: ticketSection(.inReview, id: "plan-in-review", rows: sections.inReview)
            case .building: ticketSection(.building, id: "plan-building", rows: sections.building)
            case .waiting: waiting
            case .onLandedLanding: landedLanding
            case .agents: agents
            case .themes: themes
            }
        }
        if hook.pushesLists, hook.outline != nil { browse }
    }

    // MARK: Rows

    private func ticketRow(_ row: PlanSections.TicketRow, rank: Int? = nil) -> some View {
        PlanTicketRow(
            line: row.line, theme: plan.theme(of: row.line), rank: rank, now: sections.now, waitsOnOwner: row.waitsOnOwner,
            agent: row.agent, landedLanding: row.landedLanding
        ) { hook.onOpen(.ticket(row.line.task)) }
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
                ForEach(sections.landings) { card in
                    PlanLandingRow(card: card, ci: plan.ci(of: card.landing), now: sections.now)
                    ForEach(card.tickets) { ticket in
                        PlanTicketRow(
                            line: ticket.line, theme: plan.theme(of: ticket.line), rank: nil, now: sections.now,
                            waitsOnOwner: ticket.waitsOnOwner, standing: ticket.waitsOnOwner ? "" : ticket.note ?? ""
                        ) { hook.onOpen(.ticket(ticket.line.task)) }
                        .padding(.leading, PaneMetrics.card)
                    }
                }
            }
        } header: {
            PlanHeader(title: PlanSections.Kind.landing.title, count: sections.count(.landing) == 0 ? nil : sections.count(.landing))
                .explainedOnLongPress([PlanWords.landingExplanation])
                .accessibilityIdentifier("plan-landing")
        }
    }

    // MARK: In Review and Building

    private func ticketSection(_ kind: PlanSections.Kind, id: String, rows: [PlanSections.TicketRow]) -> some View {
        Section {
            if let words = sections.emptyWords(kind) {
                empty(words, id: id)
            } else {
                ForEach(rows) { ticketRow($0) }
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
                        ticketRow(row, rank: group.rank(at: index))
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

    // MARK: Waiting for the Next Landing

    private var landedLanding: some View {
        Section {
            if landedOpen { ForEach(sections.onLandedLanding) { ticketRow($0) } }
        } header: {
            PlanHeader(title: PlanSections.Kind.onLandedLanding.title, count: sections.onLandedLanding.count, open: landedOpen)
                .contentShape(.rect)
                .onTapGesture { withAnimation { landedOpen.toggle() } }
                .accessibilityElement(children: .ignore)
            .accessibilityLabel("\(PlanSections.Kind.onLandedLanding.title), \(sections.onLandedLanding.count)")
            .accessibilityValue(landedOpen ? "Expanded" : "Collapsed")
            .accessibilityAddTraits(.isButton)
            .accessibilityIdentifier("plan-on-landed-landing")
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
                    PlanAgentRow(agent: agent, now: sections.now) { hook.onOpen(.ticket($0.task)) }
                }
            }
        } header: {
            PlanHeader(title: PlanSections.Kind.agents.title, count: nil).accessibilityIdentifier("plan-agents")
        }
    }

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

    // MARK: Themes and Tickets, pushed

    private var browse: some View {
        Section {
            // Not where the tree lists the themes already (ov-616).
            if hook.kinds?.contains(.themes) ?? true {
                NavigationLink {
                    PlanBrowseList(hook: hook, kind: .themes)
                } label: {
                    Label("Themes", systemImage: "square.grid.2x2").foregroundStyle(.primary)
                }
                .accessibilityIdentifier("plan-browse-themes")
            }
            NavigationLink {
                PlanBrowseList(hook: hook, kind: .tickets)
            } label: {
                Label("Tickets", systemImage: "point.topleft.down.to.point.bottomright.curvepath").foregroundStyle(.primary)
            }
            .accessibilityIdentifier("plan-browse-tickets")
        }
    }
}

/// One agent that exists, working or idle: who (role and model), what for and
/// its state, and its tokens. A working agent has a filled dot, an idle one a
/// ring; a tap opens its first ticket.
struct PlanAgentRow: View {
    let agent: PlanAgent
    let now: Int64
    let onOpen: (PlanAgentTicket) -> Void
    @ScaledMetric(relativeTo: .subheadline) private var markWidth: CGFloat = 22
    @Environment(\.dynamicTypeSize) private var type

    var body: some View {
        let row = content
        Group {
            if let first = agent.tickets.first {
                Button { onOpen(first) } label: { row }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(.isButton)
            } else {
                row
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel([agent.who, agent.model.isEmpty ? nil : "Model ID \(agent.model)", agent.second(now: now), agent.tokens().map { "\($0) tokens" }].compactMap { $0 }.joined(separator: ", "))
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
                // At accessibility sizes the who and the state wrap, and the
                // tokens go under them: beside them, both cut to "Orchestrato…".
                Text(agent.who)
                    .font(.subheadline.weight(.medium))
                    .lineLimit(type.isAccessibilitySize ? nil : 1)
                    .truncationMode(.tail)
                    .fixedSize(horizontal: false, vertical: type.isAccessibilitySize)
                Text(agent.second(now: now))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(type.isAccessibilitySize ? nil : 2)
                    .fixedSize(horizontal: false, vertical: true)
                if type.isAccessibilitySize, let tokens = agent.tokens() {
                    Text(tokens)
                        .font(.footnote.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 4)
            if !type.isAccessibilitySize, let tokens = agent.tokens() {
                Text(tokens)
                    .font(.footnote.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .fixedSize()
            }
            if !agent.tickets.isEmpty {
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
        }
        .contentShape(.rect)
    }
}
