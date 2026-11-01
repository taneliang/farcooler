import SwiftUI

// The rows of the phone's plan column (ov-874): see `PhonePlanColumn`.

/// A section's heading: small capitals and its count, as the Mac's.
struct PhonePlanHeading: View {
    let title: String
    var count: String?
    /// The plan's own sections are in the ink; a stage inside one is quiet.
    var prominent = true

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Text(title).font(.caption.weight(.semibold)).textCase(.uppercase)
            if let count { Text(count).font(.callout.monospacedDigit()) }
        }
        .foregroundStyle(prominent ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// "2 need you  1 stuck, 47 min  Next landing: 4 tickets, 14 min  282 of 339 done".
struct PhonePlanFacts: View {
    let facts: [PlanColumn.Fact]
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        if !facts.isEmpty {
            Section {
                // Each fact whole; the line breaks between them, never inside one.
                ChipFlow(spacing: 16) {
                    ForEach(facts) { fact in
                        fact.runs.reduce(Text("")) { line, run in
                            line + Text(run.text).fontWeight(run.strong ? .semibold : .regular)
                        }
                        .monospacedDigit()
                        .foregroundStyle(fact.attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
                        .fixedSize()
                        .accessibilityIdentifier("plan-fact-\(fact.kind.rawValue)")
                    }
                }
                .font(.subheadline)
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("plan-facts")
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 0, leading: 4, bottom: 0, trailing: 4))
            }
        }
    }
}

/// One Needs You line: a dot, its title, what it asks, how long it's waited.
struct PhoneNeedsYouLine: View {
    let title: String
    let detail: String
    let since: Date?
    let quote: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: quote) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Circle().fill(Tint.attention(scheme)).frame(width: 7, height: 7)
                    .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                    .accessibilityHidden(true)
                (Text(title).fontWeight(.medium) + Text(detail.isEmpty ? "" : "  \(detail)").foregroundStyle(.secondary))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
                if let since {
                    Text(since, format: .relative(presentation: .numeric, unitsStyle: .narrow))
                        .font(.footnote.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
            .font(.subheadline)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityHint("Puts this in the orchestrator’s message")
    }
}

/// Initiatives: each a soft card, name and progress on line 1, Next on line 2.
/// Those not moving fold into one quiet line.
struct PhonePlanInitiatives: View {
    let column: PlanColumn
    let open: (String) -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        if column.initiativeCount > 0 {
            if column.initiatives.isEmpty { Section { quiet } header: { heading } }
            ForEach(Array(column.initiatives.enumerated()), id: \.element.id) { index, initiative in
                Section {
                    row(initiative)
                } header: {
                    if index == 0 { heading }
                } footer: {
                    if index == column.initiatives.count - 1, column.quietLine != nil { quiet.padding(.top, Spacing.group) }
                }
            }
        }
    }

    private var heading: some View {
        PhonePlanHeading(title: "Initiatives", count: "\(column.initiativeCount)").accessibilityIdentifier("plan-initiatives")
    }

    @ViewBuilder private var quiet: some View {
        if let line = column.quietLine {
            Text(line).font(.footnote).foregroundStyle(.secondary).accessibilityIdentifier("plan-initiatives-quiet")
        }
    }

    private func row(_ initiative: PlanColumn.Initiative) -> some View {
        Button { open(initiative.id) } label: {
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: Spacing.inset) {
                    Text(initiative.name).font(.headline).frame(maxWidth: .infinity, alignment: .leading)
                    Text(initiative.progress).font(.footnote.monospacedDigit()).foregroundStyle(.secondary)
                }
                if let next = initiative.next {
                    Text(next)
                        .font(.subheadline)
                        .foregroundStyle(initiative.asksOwner ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
                }
                ProgressView(value: initiative.fraction)
                    .tint(.secondary)
                    .padding(.top, 2)
                    .accessibilityHidden(true)
            }
            .fixedSize(horizontal: false, vertical: true)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(initiative.name), \(initiative.progress) done. \(initiative.next ?? "")")
        .accessibilityHint("Opens the initiative")
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("plan-initiative-\(initiative.name)")
    }
}

/// The pipeline: Stuck, Building, Reviewing, Landing, each stage its agents'
/// cards with their tickets under them.
struct PhonePlanPipeline: View {
    let column: PlanColumn
    let actions: PhonePlanActions

    var body: some View {
        if !column.pipeline.isEmpty {
            Section {
            } header: {
                PhonePlanHeading(title: "Pipeline", count: column.agentWords).accessibilityIdentifier("plan-pipeline")
            }
            ForEach(column.pipeline) { group in
                Section {
                } header: {
                    PhonePlanHeading(title: group.stage.title, count: "\(group.count)", prominent: false)
                        .accessibilityIdentifier("plan-stage-\(group.stage.rawValue)")
                }
                ForEach(group.agents) { agent in
                    PhoneAgentCard(
                        agent: agent, moreWords: agent.more > 0 ? group.unattendedWords : nil, actions: actions)
                }
                if group.agents.isEmpty, let words = group.unattendedWords {
                    Section {
                        Text(words).font(.subheadline).foregroundStyle(.secondary)
                            .accessibilityIdentifier("plan-more-\(group.stage.rawValue)")
                        ForEach(group.unattended) { PhoneTicketRow(ticket: $0, open: actions.openTicket) }
                    }
                } else if !group.agents.isEmpty, !group.agents.contains(where: { $0.more > 0 }), let words = group.unattendedWords {
                    Text(words).font(.footnote).foregroundStyle(.secondary)
                        .listRowBackground(Color.clear)
                        .accessibilityIdentifier("plan-more-\(group.stage.rawValue)")
                }
            }
        }
    }
}

/// One agent: its glyph (which shows it is working), its name and what it does
/// now, then its tickets indented under it, all on one soft card.
struct PhoneAgentCard: View {
    let agent: PlanColumn.Agent
    var moreWords: String?
    let actions: PhonePlanActions
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Section {
            Button { actions.openAgent(agent) } label: { header }
                .buttonStyle(.plain)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("\(agent.name), \(agent.doing)\(agent.time.map { ", \($0)" } ?? "")")
                .accessibilityHint(agent.terminal == nil ? "Opens its ticket" : "Opens its chat")
                .accessibilityAddTraits(.isButton)
                .accessibilityIdentifier("plan-agent-\(agent.id)")
            ForEach(agent.tickets) { PhoneTicketRow(ticket: $0, open: actions.openTicket) }
            if let moreWords {
                Text(moreWords)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .padding(.leading, PhoneTicketRow.indent)
                    .accessibilityIdentifier("plan-more-\(agent.id)")
            }
        }
    }

    private var header: some View {
        let trouble = agent.doingIsTrouble
        return HStack(alignment: .firstTextBaseline, spacing: 10) {
            // Activity is the agent's: a quiet pulse while it works, amber and
            // still when stuck, still when idle.
            Image(systemName: agent.role.symbol)
                .font(.callout)
                .foregroundStyle(trouble ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
                .symbolEffect(.pulse, options: .repeating.speed(0.4), isActive: agent.working && !trouble)
                .frame(width: 20)
                .accessibilityIdentifier("plan-agent-glyph-\(agent.id)")
            (Text(agent.name).fontWeight(.semibold) + Text("  ")
                + Text(agent.doing).foregroundStyle(trouble ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.primary)))
                .font(.subheadline)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
            if let time = agent.time {
                Text(time).font(.footnote.monospacedDigit()).foregroundStyle(.secondary)
            }
        }
        .contentShape(.rect)
    }
}

/// A ticket under its agent: its key in monospace, its title, its initiative.
struct PhoneTicketRow: View {
    /// Past the role glyph, so the ticket reads as the agent's.
    static let indent: CGFloat = 30

    let ticket: PlanColumn.Ticket
    let open: (String) -> Void
    #if DEBUG
        /// How wide its key is drawn, which a test reads off `plan-ticket-key-<key>`
        /// to tell monospace from the system font; the row itself is one element.
        @State private var keyWidth: CGFloat = 0
    #endif

    var body: some View {
        Button { open(ticket.task) } label: {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text(ticket.key)
                    .font(.footnote.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .fixedSize()
                    #if DEBUG
                        .background {
                            GeometryReader { box in
                                Color.clear.preference(key: KeyWidth.self, value: box.size.width)  // style-exempt: DEBUG probe
                            }
                        }
                    #endif
                VStack(alignment: .leading, spacing: 1) {
                    Text(ticket.title).font(.subheadline)
                    if let initiative = ticket.initiative {
                        Text(initiative).font(.footnote).foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.leading, Self.indent)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(ticket.key), \(ticket.title)\(ticket.initiative.map { ", for \($0)" } ?? ""), \(ticket.stage.title)")
        .accessibilityHint("Opens the ticket")
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("plan-ticket-\(ticket.key)")
        #if DEBUG
            .onPreferenceChange(KeyWidth.self) { keyWidth = $0 }
            .overlay(alignment: .leading) {
                Color.clear  // style-exempt: DEBUG probe, no fill
                    .frame(width: keyWidth, height: 1)
                    .padding(.leading, Self.indent)
                    .accessibilityElement()
                    .accessibilityIdentifier("plan-ticket-key-\(ticket.key)")
            }
        #endif
    }
}

#if DEBUG
    private struct KeyWidth: PreferenceKey {
        static let defaultValue: CGFloat = 0
        static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
    }
#endif

/// "10 tickets waiting for a builder, 22 not started, 796 done". Each count opens
/// the list of its tickets.
struct PhonePlanFooter: View {
    let counts: [PlanColumn.Count]
    let open: (PlanColumn.Count) -> Void

    var body: some View {
        if !counts.isEmpty {
            Section {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    ForEach(counts) { count in
                        // Each count opens its tickets (ov-874): a ticket whose
                        // initiative is quiet is reached here.
                        Button { open(count) } label: {
                            HStack(spacing: 4) {
                                Text(count.words)
                                Image(systemName: "chevron.forward").font(.caption2.weight(.semibold)).accessibilityHidden(true)
                            }
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("plan-footer-\(count.kind.rawValue)")
                    }
                }
                .font(.footnote.monospacedDigit())
                .foregroundStyle(.secondary)
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("plan-footer")
                .listRowBackground(Color.clear)
            }
        }
    }
}
