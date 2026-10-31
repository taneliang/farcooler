import AgentKit
import AppKit
import SwiftUI

// The plan column (ov-867, C10f): the facts line, Needs You, Initiatives,
// the pipeline and the footer, on the window's plane beside the
// orchestrator. The words and rules are AgentKit's `PlanColumn`; this only
// lays them out, natively: text styles that follow the system's size, the
// system's hover wash, and no rules. Type, space and the soft fill of each
// agent's card do the grouping.
//
// Nothing here acts on an agent. Stopping, assigning and answering are said
// to the orchestrator: a Needs You row quotes itself into its input.

/// What the column's rows do in this window.
struct PlanColumnActions {
    /// Put a reference to a Needs You row in the orchestrator's input.
    var quote: (String) -> Void = { _ in }
    /// Open an agent's chat (its pane), else its lane's page.
    var openAgent: (PlanColumn.Agent) -> Void = { _ in }
    /// Open a ticket.
    var openTicket: (String) -> Void = { _ in }
    /// The same, from under an agent's card, which the page's breadcrumb names
    /// (ov-873). Falls back to `openTicket`.
    var openTicketUnder: ((PlanColumn.Agent, String) -> Void)?

    /// A ticket on an agent's card opened.
    func open(ticket: String, under agent: PlanColumn.Agent) {
        if let openTicketUnder { openTicketUnder(agent, ticket) } else { openTicket(ticket) }
    }
    /// Open the board's list for a status.
    var openList: (TaskStatus) -> Void = { _ in }

    /// The same, each putting `close` first: the peek goes before anything
    /// opens.
    func closing(_ close: @escaping () -> Void) -> PlanColumnActions {
        PlanColumnActions(
            quote: { close(); quote($0) }, openAgent: { close(); openAgent($0) },
            openTicket: { close(); openTicket($0) },
            openTicketUnder: openTicketUnder.map { under in { agent, ticket in close(); under(agent, ticket) } },
            openList: { close(); openList($0) })
    }
}

/// The plan column: what the canvas's home draws.
struct PlanColumnView: View {
    @ObservedObject var plan: PlanStore
    let statuses: [String: TaskStatus]
    /// Each card's title on the board, for a ticket the plan doesn't carry.
    var titles: [String: String] = [:]
    let needsYou: PlanNeedsYou
    let actions: PlanColumnActions
    let onOpen: (PlanPage) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if plan.hasRead, plan.reconnecting { PlanReconnecting() }
            if plan.hasRead, let line = plan.staleLine { PlanStale(line: line) { Task { await plan.tryAgain() } } }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .identified("plan-column")
        .task(id: plan.generation) { await plan.reloadIfMoved() }
        .task(id: ObjectIdentifier(plan)) { await plan.readIfNeverRead() }
        .onAppear { plan.attach() }
        .onDisappear { plan.detach() }
    }

    /// The themes asking for the owner, counted with Needs You.
    private var asks: [PlanTheme] { plan.plan.shownThemes.filter { !$0.ownerAsk.isEmpty } }

    @ViewBuilder private var content: some View {
        if !plan.available {
            needs
            if needsYou.items.isEmpty { PlanNotice(title: PlanWords.needsUpdate, detail: nil).identified("plan-needs-update") }
        } else if !plan.hasRead {
            needs
            if plan.reconnecting {
                PlanReconnecting()
            } else if let trouble = plan.trouble {
                VStack(alignment: .leading, spacing: Spacing.group) {
                    Text(trouble).foregroundStyle(.quietInk)
                    Button("Try Again") { Task { await plan.tryAgain() } }
                }
            } else {
                ProgressView().controlSize(.small).frame(maxWidth: .infinity, minHeight: 80)
            }
        } else if plan.plan.isEmpty && needsYou.items.isEmpty {
            PlanEmptyNotice().identified("plan-empty")
        } else {
            let column = PlanColumn(
                plan: plan.plan, needsYou: needsYou.items.count + asks.count, statuses: statuses, titles: titles)
            PlanFactsLine(facts: column.facts)
            needs
            PlanInitiativesSection(column: column, onOpen: onOpen)
            PlanPipelineSection(column: column, actions: actions)
            PlanFooterLine(counts: column.footer, open: actions.openList)
        }
    }

    /// Needs You: one quiet line each, most urgent first. A click quotes it
    /// into the orchestrator's input; it's answered there, or on the Needs
    /// You page.
    @ViewBuilder private var needs: some View {
        let count = needsYou.items.count + asks.count
        if count > 0 {
            PlanColumnHeading(title: "Needs You", count: "\(count)", prominent: true)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(asks) { theme in
                    PlanNeedsYouLine(
                        title: theme.name, detail: theme.ownerAsk, since: nil,
                        quote: { actions.quote(PlanColumn.quote(key: nil, title: "\(theme.name): \(theme.ownerAsk)")) })
                    .identified("plan-needs-theme-\(theme.name)")
                }
                ForEach(needsYou.items) { item in
                    let title = item.task?.title ?? item.question
                    let detail = item.task == nil ? NeedsYouRowModel.subject(item) : item.question
                    PlanNeedsYouLine(
                        title: title, detail: detail, since: item.since,
                        quote: { actions.quote(PlanColumn.quote(key: item.task?.key, title: title)) })
                    .identified("plan-needs-\(item.key)")
                }
            }
            .identified("plan-needs-you")
        }
    }
}

/// The column's rhythm, from C10f's 4/8-point grid.
enum PlanColumnMetrics {
    /// The gutter each side of the column.
    static let gutter: CGFloat = 28
    /// Above a section's heading.
    static let section: CGFloat = 32
    /// Above a stage's heading inside the pipeline.
    static let stage: CGFloat = 20
    /// Between agent cards.
    static let cards: CGFloat = 8
    /// A ticket's indent inside its card: past the role glyph.
    static let ticketIndent: CGFloat = 44
    /// The initiative's meter.
    static let meter: CGFloat = 80
    /// The column the right-hand numbers sit in.
    static let figures: CGFloat = 56
}

/// A section's heading: small capitals, and its count.
struct PlanColumnHeading: View {
    let title: String
    var count: String?
    /// The plan's own sections are in the ink; a stage inside one is quiet.
    var prominent = false
    var top: CGFloat = PlanColumnMetrics.section

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Text(title)
                .font(.caption.weight(.semibold))
                .textCase(.uppercase)
                .kerning(0.5)
                .foregroundStyle(prominent ? AnyShapeStyle(.primary) : AnyShapeStyle(.quietInk))
            if let count {
                Text(count)
                    .font(.callout.monospacedDigit())
                    .foregroundStyle(prominent ? AnyShapeStyle(.primary) : AnyShapeStyle(.quietInk))
            }
        }
        .padding(.top, top)
        .padding(.bottom, Spacing.group)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// "2 need you   1 stuck, 47 min   Next landing: 6 tickets, 14 min   282 of 317 done".
struct PlanFactsLine: View {
    let facts: [PlanColumn.Fact]
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        // Each fact whole; the line breaks between them, never inside one.
        PlanFlowLayout(spacing: 20, lineSpacing: Spacing.tight) {
            ForEach(facts) { fact in
                fact.runs.reduce(Text("")) { line, run in
                    line + Text(run.text).fontWeight(run.strong ? .semibold : .regular)
                }
                .monospacedDigit()
                .foregroundStyle(fact.attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
                .fixedSize()
                .identified("plan-fact-\(fact.kind.rawValue)")
            }
        }
        .font(.body)
        .identified("plan-facts")
    }
}

/// One Needs You line: a dot, its title, what it asks, how long it's waited.
struct PlanNeedsYouLine: View {
    let title: String
    let detail: String
    let since: Date?
    let quote: () -> Void
    @State private var hovering = false
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: quote) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Circle().fill(Tint.attention(scheme)).frame(width: 7, height: 7)
                    .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                (Text(title).fontWeight(.medium)
                    + Text(detail.isEmpty ? "" : "   \(detail)").foregroundStyle(.quietInk))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
                if let since {
                    Text(since, format: .relative(presentation: .numeric, unitsStyle: .narrow))
                        .font(.callout.monospacedDigit())
                        .foregroundStyle(.quietInk)
                }
            }
            .font(.body)
            .padding(.vertical, Spacing.tight)
            .padding(.horizontal, Spacing.group)
            .background { if hovering { RoundedRectangle.control.fill(Fill.hover) } }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .padding(.horizontal, -Spacing.group)
        .onHover { hovering = $0 }
        .help("Quote this in the orchestrator’s input")
        .accessibilityHint("Quotes this in the orchestrator’s input")
    }
}

/// Initiatives: the larger goals, each with its goal, progress and next.
struct PlanInitiativesSection: View {
    let column: PlanColumn
    let onOpen: (PlanPage) -> Void

    var body: some View {
        if column.initiativeCount > 0 {
            PlanColumnHeading(title: "Initiatives", count: "\(column.initiativeCount)", prominent: true)
            VStack(alignment: .leading, spacing: PlanColumnMetrics.cards) {
                ForEach(column.initiatives) { initiative in
                    PlanInitiativeRow(initiative: initiative) { onOpen(.theme(initiative.id)) }
                }
                if let quiet = column.quietLine {
                    Text(quiet)
                        .font(.callout)
                        .foregroundStyle(.quietInk)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, Spacing.tight)
                        .padding(.horizontal, Spacing.group)
                        .identified("plan-initiatives-quiet")
                }
            }
            .identified("plan-initiatives")
        }
    }
}

struct PlanInitiativeRow: View {
    let initiative: PlanColumn.Initiative
    let open: () -> Void
    @State private var hovering = false
    @Environment(\.colorScheme) private var scheme

    /// Name and progress on one baseline, then what's next: the line that
    /// matters most. The goal is on the initiative's page and in the help.
    var body: some View {
        Button(action: open) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: Spacing.inset) {
                    Text(initiative.name)
                        .font(.headline)
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    ProgressView(value: initiative.fraction)
                        .progressViewStyle(.linear)
                        .controlSize(.small)
                        .tint(.secondary)
                        .frame(width: PlanColumnMetrics.meter)
                        .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                        .accessibilityHidden(true)
                    Text(initiative.progress)
                        .font(.callout.monospacedDigit())
                        .foregroundStyle(.quietInk)
                        .frame(minWidth: PlanColumnMetrics.figures - 8, alignment: .trailing)
                }
                if let next = initiative.next {
                    Text(next)
                        .font(.body)
                        .foregroundStyle(initiative.asksOwner ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(.vertical, Spacing.inset)
            .padding(.horizontal, Spacing.section)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background { if hovering { RoundedRectangle.control.fill(Fill.hover) } }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .surface(.content, in: .card, fill: PlanColumnMetrics.cardFill)
        .onHover { hovering = $0 }
        .help(initiative.goal.isEmpty ? "Open \(initiative.name)" : initiative.goal)
        .accessibilityLabel("\(initiative.name), \(initiative.progress) done. \(initiative.next ?? "")")
        .accessibilityHint(initiative.goal)
        .identified("plan-initiative-\(initiative.name)")
    }
}

/// The pipeline: Stuck, Building, Reviewing, Landing, each a stack of agent
/// cards with their tickets under them.
struct PlanPipelineSection: View {
    let column: PlanColumn
    let actions: PlanColumnActions

    var body: some View {
        if !column.pipeline.isEmpty {
            PlanColumnHeading(title: "Pipeline", count: column.agentWords, prominent: true)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(column.pipeline.enumerated()), id: \.element.id) { index, group in
                    PlanColumnHeading(
                        title: group.stage.title, count: "\(group.count)", top: index == 0 ? 0 : PlanColumnMetrics.stage)
                    VStack(alignment: .leading, spacing: PlanColumnMetrics.cards) {
                        ForEach(group.agents) { agent in
                            PlanAgentCard(agent: agent, stage: group.stage, moreWords: agent.more > 0 ? group.unattendedWords : nil, actions: actions)
                        }
                        if group.agents.isEmpty, let words = group.unattendedWords {
                            PlanUnattendedCard(words: words, tickets: group.unattended, actions: actions)
                        } else if !group.agents.isEmpty, !group.agents.contains(where: { $0.more > 0 }),
                            let words = group.unattendedWords
                        {
                            // No card has tickets to hang it on: its own line.
                            Text(words)
                                .font(.callout)
                                .foregroundStyle(.quietInk)
                                .padding(.horizontal, Spacing.group)
                                .identified("plan-more-\(group.stage.rawValue)")
                        }
                    }
                    .identified("plan-stage-\(group.stage.rawValue)")
                }
            }
            .identified("plan-pipeline")
        }
    }
}

/// One agent: its role, name, what it does now and for how long, then its
/// tickets, on a soft fill. The header opens its chat; a ticket opens itself.
struct PlanAgentCard: View {
    let agent: PlanColumn.Agent
    let stage: PlanColumn.Stage
    /// "12 more waiting for review", under the last card of a stage.
    var moreWords: String?
    let actions: PlanColumnActions
    @State private var hovering = false
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button { actions.openAgent(agent) } label: { header }
                .buttonStyle(.plain)
                .onHover { hovering = $0 }
                .help(agent.terminal == nil ? "Open its lane" : "Open \(agent.name)’s chat")
                .accessibilityLabel("\(agent.name), \(agent.doing)\(agent.time.map { ", \($0)" } ?? "")")
                .accessibilityHint(agent.terminal == nil ? "Opens its lane" : "Opens its chat")
                .identified("plan-agent-\(agent.id)")
            ForEach(agent.tickets) { ticket in
                PlanTicketRow(ticket: ticket) { actions.open(ticket: ticket.task, under: agent) }
            }
            if let moreWords {
                Text(moreWords)
                    .font(.callout)
                    .foregroundStyle(.quietInk)
                    .padding(.leading, PlanColumnMetrics.ticketIndent)
                    .padding(.trailing, Spacing.section)
                    .padding(.vertical, Spacing.tight)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.top, Spacing.tight)
        .padding(.bottom, Spacing.group)
        .frame(maxWidth: .infinity, alignment: .leading)
        .surface(.content, in: .card, fill: PlanColumnMetrics.cardFill)
    }

    private var header: some View {
        let trouble = agent.doingIsTrouble ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.primary)
        return HStack(alignment: .firstTextBaseline, spacing: 10) {
            // Activity is the agent's: a quiet pulse while it works, amber and
            // still when stuck, still when idle.
            Image(systemName: agent.role.symbol)
                .font(.callout)
                .foregroundStyle(agent.doingIsTrouble ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
                .symbolEffect(.pulse, options: .repeating.speed(0.4), isActive: agent.working && !agent.doingIsTrouble)
                .frame(width: 16)
                .accessibilityHidden(true)
                .probed("plan-agent-glyph-\(agent.id)")
            (Text(agent.name).fontWeight(.semibold) + Text("  ") + Text(agent.doing).foregroundStyle(trouble))
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
            if let time = agent.time {
                Text(time)
                    .font(.callout.monospacedDigit())
                    .foregroundStyle(.quietInk)
            }
        }
        .font(.body)
        .padding(.horizontal, Spacing.group)
        .padding(.vertical, Spacing.tight)
        .background { if hovering { RoundedRectangle.control.fill(Fill.hover) } }
        .contentShape(.rect)
        .padding(.horizontal, Spacing.group)
        .padding(.top, Spacing.tight)
    }
}

/// Tickets in a stage with no agent on them: no header, since no agent is
/// there to name.
struct PlanUnattendedCard: View {
    let words: String
    let tickets: [PlanColumn.Ticket]
    let actions: PlanColumnActions

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(words)
                .font(.callout)
                .foregroundStyle(.quietInk)
                .padding(.horizontal, Spacing.section)
                .padding(.vertical, Spacing.tight)
                .padding(.top, Spacing.tight)
            ForEach(tickets) { ticket in PlanTicketRow(ticket: ticket) { actions.openTicket(ticket.task) } }
        }
        .padding(.bottom, Spacing.group)
        .frame(maxWidth: .infinity, alignment: .leading)
        .surface(.content, in: .card, fill: PlanColumnMetrics.cardFill)
    }
}

/// A ticket under its agent: a quiet dot, its title, its initiative.
struct PlanTicketRow: View {
    let ticket: PlanColumn.Ticket
    let open: () -> Void
    @State private var hovering = false
    /// Room for "ov-0000" at the callout size, which follows the system's text size.
    @ScaledMetric(relativeTo: .callout) private var keyWidth: CGFloat = 46

    var body: some View {
        Button(action: open) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                // Its key, in secondary ink at one width so every title starts
                // on one edge; the ticket never reads as progress (ov-873).
                Text(ticket.key)
                    .font(.callout.monospaced())
                    .foregroundStyle(.quietInk)
                    .lineLimit(1)
                    .fixedSize()
                    .frame(minWidth: keyWidth, alignment: .leading)
                    .accessibilityHidden(true)
                    .probed("plan-ticket-key-\(ticket.key)")
                Text(ticket.title)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
                if let initiative = ticket.initiative {
                    Text(initiative)
                        .font(.callout)
                        .foregroundStyle(.quietInk)
                        .fixedSize()
                }
            }
            .font(.body)
            .padding(.vertical, 3)
            .padding(.leading, PlanColumnMetrics.ticketIndent - Spacing.group)
            .padding(.trailing, Spacing.group)
            .background { if hovering { RoundedRectangle.control.fill(Fill.hover) } }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        // The wash sits a step in from the card's edges.
        .padding(.horizontal, Spacing.group)
        .onHover { hovering = $0 }
        .accessibilityLabel("\(ticket.key), \(ticket.title)\(ticket.initiative.map { ", for \($0)" } ?? ""), \(ticket.stage.title)")
        .accessibilityHint("Opens the ticket")
        .identified("plan-ticket-\(ticket.key)")
    }
}

/// "10 tickets waiting for a builder, 22 not started, 796 done": each opens
/// its list.
struct PlanFooterLine: View {
    let counts: [PlanColumn.Count]
    let open: (TaskStatus) -> Void

    var body: some View {
        if !counts.isEmpty {
            PlanFlowLayout(spacing: 0, lineSpacing: Spacing.tight) {
                ForEach(Array(counts.enumerated()), id: \.element.id) { index, count in
                    Button { open(count.status) } label: {
                        Text(count.words + (index < counts.count - 1 ? ", " : ""))
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.quietInk)
                    .pointerStyle(.link)
                    .help("Open the list")
                    .identified("plan-footer-\(count.kind.rawValue)")
                }
            }
            .font(.callout.monospacedDigit())
            .padding(.top, 24)
            .identified("plan-footer")
        }
    }
}

/// Views in a row that wrap to the next line as a whole: a fact never
/// breaks inside itself.
struct PlanFlowLayout: Layout {
    var spacing: CGFloat
    var lineSpacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        let height = rows.reduce(CGFloat(0)) { $0 + $1.height } + lineSpacing * CGFloat(max(0, rows.count - 1))
        let width = rows.map(\.width).max() ?? 0
        return CGSize(width: proposal.width ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for index in row.items {
                let size = subviews[index].sizeThatFits(.unspecified)
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + lineSpacing
        }
    }

    private struct Row { var items: [Int] = []; var width: CGFloat = 0; var height: CGFloat = 0 }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        var row = Row()
        for (index, subview) in subviews.enumerated() {
            let size = subview.sizeThatFits(.unspecified)
            let next = row.items.isEmpty ? size.width : row.width + spacing + size.width
            if !row.items.isEmpty, next > width {
                rows.append(row)
                row = Row()
            }
            row.width = row.items.isEmpty ? size.width : row.width + spacing + size.width
            row.height = max(row.height, size.height)
            row.items.append(index)
        }
        if !row.items.isEmpty { rows.append(row) }
        return rows
    }
}

extension PlanColumnMetrics {
    /// An agent card's soft fill, from the system's own colors: the control
    /// background (white) on the light plane, and the quaternary system fill
    /// in dark, a step lighter than the plane as a grouped box is there.
    static let cardFill = Color(
        nsColor: NSColor(name: "PlanCardFill") { appearance in
            appearance.bestMatch(from: [.darkAqua, .vibrantDark, .aqua]) == .aqua
                ? .controlBackgroundColor : .quaternarySystemFill
        })
}
