import SwiftUI

// The Plan's sections on a phone and in the iPad's plan column (ov-503,
// design 2 and 7; ov-522): Landing, In Review, Building, Waiting, Waiting for the Next Landing
// (folded), Agents, then Themes as a grid and, on a phone, Themes and Tickets
// as two rows that push lists. The Rust core's view (`PlanView`, ov-1016)
// decides what is where, every word and what each says empty; this lays it
// out as an inset list's sections, one column.

struct PlanSectionsList: View {
    let view: PlanView
    let plan: PlanModel
    let hook: PlanBoardHook

    @State private var landedOpen: Bool
    @Environment(\.colorScheme) private var scheme

    init(view: PlanView, plan: PlanModel, hook: PlanBoardHook) {
        self.view = view
        self.plan = plan
        self.hook = hook
        _landedOpen = State(initialValue: !(view.section(.onLandedLanding)?.folded ?? true))
    }

    var body: some View {
        // In the view's order, so what is drawn is what the model says, in the
        // order it says (owner decision 7; ov-522).
        ForEach(view.sections.filter { hook.kinds?.contains($0.kind) ?? true }) { section in
            switch section.kind {
            case .needsYou, .unknown: EmptyView()
            case .landing: landing(section)
            case .inReview: cardSection(section, id: "plan-in-review", cards: view.inReview)
            case .building: cardSection(section, id: "plan-building", cards: view.building)
            case .waiting: waiting(section)
            case .onLandedLanding: landedLanding(section)
            case .agents: agents(section)
            case .themes: themes(section)
            }
        }
        if hook.pushesLists, hook.outline != nil { browse }
    }

    // MARK: Rows

    private func cardRow(_ card: PlanView.TicketCard) -> some View {
        PlanCardRow(card: card) { hook.onOpen(.ticket(card.id)) }
    }

    /// What a section says with nothing in it.
    private func empty(_ words: String, id: String) -> some View {
        Text(words)
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .accessibilityIdentifier("\(id)-empty")
    }

    /// A section's title and count: none drawn for none.
    private func header(_ section: PlanView.Section) -> PlanHeader {
        PlanHeader(title: section.title, count: section.count == 0 ? nil : section.count)
    }

    // MARK: Landing

    private func landing(_ section: PlanView.Section) -> some View {
        Section {
            if let words = section.empty {
                empty(words, id: "plan-landing")
            } else {
                ForEach(view.landings) { card in
                    PlanLandingRow(card: card, explanation: section.help, ci: ci(card), now: view.now)
                    ForEach(card.tickets) { ticket in
                        PlanCardRow(ticket: ticket, theme: plan.theme(ofTask: ticket.id)) { hook.onOpen(.ticket(ticket.id)) }
                            .padding(.leading, PaneMetrics.card)
                    }
                }
            }
        } header: {
            header(section)
                .explainedOnLongPress([section.help].compactMap { $0 })
                .accessibilityIdentifier("plan-landing")
        }
    }

    /// The CI read of a landing's pushed commit, the app's own.
    private func ci(_ card: PlanView.LandingCard) -> PlanCIRead? {
        plan.landings.first { $0.id == card.id }.flatMap { plan.ci(of: $0) }
    }

    // MARK: In Review and Building

    private func cardSection(_ section: PlanView.Section, id: String, cards: [PlanView.TicketCard]) -> some View {
        Section {
            if let words = section.empty {
                empty(words, id: id)
            } else {
                ForEach(cards) { cardRow($0) }
            }
        } header: {
            header(section).accessibilityIdentifier(id)
        }
    }

    // MARK: Waiting

    private func waiting(_ section: PlanView.Section) -> some View {
        Section {
            if let words = section.empty {
                empty(words, id: "plan-waiting")
            } else {
                ForEach(view.waiting) { group in
                    // Each group is named only when something else shares the section.
                    if view.namesWaitingGroups { groupTitle(group) }
                    ForEach(group.cards) { cardRow($0) }
                }
            }
        } header: {
            header(section).accessibilityIdentifier("plan-waiting")
        }
    }

    private func groupTitle(_ group: PlanView.WaitingGroup) -> some View {
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

    private func landedLanding(_ section: PlanView.Section) -> some View {
        Section {
            if landedOpen { ForEach(view.onLandedLanding) { cardRow($0) } }
        } header: {
            PlanHeader(title: section.title, count: section.count, open: landedOpen)
                .contentShape(.rect)
                .onTapGesture { withAnimation { landedOpen.toggle() } }
                .accessibilityElement(children: .ignore)
            .accessibilityLabel("\(section.title), \(section.count)")
            .accessibilityValue(landedOpen ? "Expanded" : "Collapsed")
            .accessibilityAddTraits(.isButton)
            .accessibilityIdentifier("plan-on-landed-landing")
        }
    }

    // MARK: Agents

    private func agents(_ section: PlanView.Section) -> some View {
        let agents = view.agents
        let opens = Dictionary(plan.agents.compactMap { a in a.tickets.first.map { (a.id, $0.task) } }, uniquingKeysWith: { first, _ in first })
        return Section {
            if let words = section.empty {
                empty(words, id: "plan-agents")
            } else {
                Text(agents.header)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("plan-agents-header")
                ForEach(agents.rows) { agent in
                    PlanAgentRow(agent: agent, opens: opens[agent.id]) { hook.onOpen(.ticket($0)) }
                }
            }
        } header: {
            PlanHeader(title: section.title, count: nil).accessibilityIdentifier("plan-agents")
        }
    }

    // MARK: Themes

    private func themes(_ section: PlanView.Section) -> some View {
        Section {
            PlanThemeGridView(grid: view.themes, plan: plan, columns: hook.tileColumns) { hook.onOpen(.theme($0)) }
                .listRowInsets(EdgeInsets())
                .listRowBackground(Color.clear)
                .listRowSeparator(.hidden)
        } header: {
            PlanHeader(title: section.title, count: section.count).accessibilityIdentifier("plan-themes")
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
    let agent: PlanView.AgentRow
    /// The task its row opens, or nil for an agent on no ticket.
    let opens: String?
    let onOpen: (String) -> Void
    @ScaledMetric(relativeTo: .subheadline) private var markWidth: CGFloat = 22
    @Environment(\.dynamicTypeSize) private var type

    var body: some View {
        let row = content
        Group {
            if let opens {
                Button { onOpen(opens) } label: { row }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(.isButton)
            } else {
                row
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(agent.accessibility)
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
                Text(agent.second)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(type.isAccessibilitySize ? nil : 2)
                    .fixedSize(horizontal: false, vertical: true)
                if type.isAccessibilitySize, let tokens = agent.tokens {
                    Text(tokens)
                        .font(.footnote.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 4)
            if !type.isAccessibilitySize, let tokens = agent.tokens {
                Text(tokens)
                    .font(.footnote.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .fixedSize()
            }
            if opens != nil {
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
        }
        .contentShape(.rect)
    }
}

/// One ticket's row, from the Rust core's card: its title and the theme it
/// serves, then its reason or state, then its notes (the pull request's stage,
/// the engine's step and gate, its warnings), each a line. In Next Up its rank
/// sits in the glyph column; elsewhere its stage's mark, amber when a note
/// needs a person. A ticket on a landing draws the same row from its landing
/// ticket: what it says of itself, and its key.
struct PlanCardRow: View {
    let key: String
    let heading: String
    let theme: String?
    let second: String
    let rank: Int?
    let mark: PlanView.Mark
    let markAttention: Bool
    let notes: [PlanView.Note]
    let accessibility: String
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme
    /// The mark's column, which grows with its glyph so the glyph never reaches the name (ov-424).
    @ScaledMetric(relativeTo: .subheadline) private var markWidth: CGFloat = 22
    /// At accessibility sizes the name, the theme and the second line wrap
    /// rather than cut off (ov-653).
    @Environment(\.dynamicTypeSize) private var type

    init(card: PlanView.TicketCard, action: @escaping () -> Void) {
        key = card.key
        heading = card.heading
        theme = card.theme
        second = card.second
        rank = card.rank
        mark = card.mark
        markAttention = card.markAttention
        notes = card.notes
        accessibility = card.accessibility
        self.action = action
    }

    init(ticket: PlanView.LandingTicket, theme: PlanTheme?, action: @escaping () -> Void) {
        key = ticket.key
        heading = ticket.heading
        self.theme = theme?.name
        second = ticket.second
        rank = nil
        mark = ticket.mark
        markAttention = ticket.markAttention || ticket.noteAttention
        // Waiting on the owner is its one warning, said under it as on any row.
        notes = (ticket.waitsOnOwner ? [PlanView.Note(kind: .warning, text: ticket.note ?? "", attention: true, help: nil)] : [])
            + ticket.notes
        accessibility = ticket.accessibility
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.card) {
                markView.frameProbe("plan-ticket-mark").frame(width: markWidth, alignment: .leading)
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
                        .lineLimit(type.isAccessibilitySize ? nil : 2)
                        .fixedSize(horizontal: false, vertical: true)
                    ForEach(Array(notes.enumerated()), id: \.offset) { _, note in noteLine(note) }
                }
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        // What VoiceOver reads is the view's: the rank, the name, the theme,
        // the reason or state, and its notes, in that order.
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibility)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("plan-ticket-\(key)")
        .probesShown()
    }

    private var name: some View {
        Text(heading)
            .font(.subheadline.weight(.medium))
            .lineLimit(type.isAccessibilitySize ? nil : 2)
            .fixedSize(horizontal: false, vertical: true)
            .layoutPriority(1)
            .frameProbe("plan-ticket-name")
    }

    @ViewBuilder private var themeName: some View {
        if let theme {
            Text(theme)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .lineLimit(type.isAccessibilitySize ? nil : 1)
                .truncationMode(.tail)
                .fixedSize(horizontal: false, vertical: type.isAccessibilitySize)
                .accessibilityIdentifier("plan-ticket-\(key)-theme")
        }
    }

    /// A note: gray, or amber for what needs a person. A pull request's stage
    /// or the engine's step keeps to one line, cut in the middle (a login may
    /// be long, ov-316).
    @ViewBuilder private func noteLine(_ note: PlanView.Note) -> some View {
        let oneLine = note.kind == .stage || note.kind == .run
        Text(note.text)
            .font(.footnote.weight(note.attention ? .medium : .regular))
            .foregroundStyle(note.attention ? AnyShapeStyle(GlancePalette.amber(scheme)) : AnyShapeStyle(.secondary))
            .lineLimit(oneLine ? 1 : nil)
            .truncationMode(.middle)
            .accessibilityIdentifier(Self.probe(note, key: key))
    }

    static func probe(_ note: PlanView.Note, key: String) -> String {
        switch note.kind {
        case .stage: note.attention ? "plan-ticket-\(key)-stage-attention" : "plan-ticket-\(key)-stage"
        case .run: "plan-ticket-\(key)-run"
        case .warning: "plan-ticket-\(key)-warning"
        case .overBudget: "plan-ticket-\(key)-over-budget"
        case .unknown: "plan-ticket-\(key)-note"
        }
    }

    @ViewBuilder private var markView: some View {
        if let rank {
            Text("\(rank)")
                .font(.subheadline.monospacedDigit())
                .foregroundStyle(.secondary)
        } else {
            Image(systemName: PlanGlyph.name(mark))
                .font(.subheadline)
                .foregroundStyle(markAttention ? AnyShapeStyle(GlancePalette.amber(scheme)) : AnyShapeStyle(.secondary))
                .accessibilityHidden(true)
        }
    }
}

extension PlanGlyph {
    /// A card's mark, as the view names it.
    static func name(_ mark: PlanView.Mark) -> String {
        switch mark {
        case .rank, .unknown: "questionmark.circle"
        case .waiting: name(.waiting)
        case .building: name(.building)
        case .fixing: name(.building, fixing: true)
        case .reviewing: name(.reviewing)
        case .needsYou: name(.needsYou)
        case .landing: name(.landing)
        case .landed: name(.landed)
        case .dropped: name(.dropped)
        }
    }
}
