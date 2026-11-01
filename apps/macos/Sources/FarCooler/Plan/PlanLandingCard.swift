import AgentKit
import SwiftUI

// A landing in Landing (ov-503, design 2; ov-522): a card headed "Landing 86"
// with its title under it and what it carries, a ladder of where it stands in
// three plain steps (each explained on hover), the CI line, and its tickets. A
// ticket aboard says nothing; it speaks only once landed or when it waits on
// the owner. Amber only for CI that failed, a landing that stalled, and a
// ticket waiting on the owner, each with words.
//
//   Landing 86
//   The team directory; the iPhone composer
//   ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬ ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬ ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬
//   Checking Here    Checking on CI  Landed
//   3f78754f · 2 of 5 jobs done · 12 min · Lander · Opus · 1.4M
//   ✓ team-dir
//   ios-overwrite                                   Needs you

struct PlanLandingCard: View {
    let card: PlanSections.LandingCard
    let ci: PlanCIRead?
    let now: Int64
    let selected: PlanPage?
    let keyed: Bool
    let onOpen: (PlanPage) -> Void
    @Environment(\.colorScheme) private var scheme
    @Environment(\.openURL) private var openURL

    private var landing: PlanLanding { card.landing }
    private var run: URL? { ci.flatMap { PageLinks.https($0.url) } }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            heading
            PlanLadderView(phase: card.phase)
            facts
            if !card.tickets.isEmpty {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(card.tickets) { ticket in
                        PlanLandingTicketRow(
                            ticket: ticket, now: now,
                            selected: selected == .ticket(ticket.id), keyed: keyed, onOpen: { onOpen(.ticket(ticket.id)) })
                    }
                }
            }
        }
        .padding(.horizontal, Spacing.inset)
        .padding(.vertical, Spacing.inset)
        .frame(maxWidth: .infinity, alignment: .leading)
        .surface(.inset, in: .card)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(accessibility)
        .identified("plan-landing-\(landing.number)")
    }

    private var heading: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
            Text(landing.label)
                .font(.system(size: WorkspaceStyle.PaneText.title, weight: .semibold))
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
                .identified("plan-landing-\(landing.number)-label")
            if let subtitle = landing.subtitle {
                Text(subtitle)
                    .font(.system(size: WorkspaceStyle.PaneText.body))
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .identified("plan-landing-\(landing.number)-subtitle")
            }
            if let carries = landing.carries {
                Text(carries)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.quietInk)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .identified("plan-landing-\(landing.number)-carries")
            }
        }
        // What a landing is, said here and nowhere in the text.
        .help(PlanWords.landingExplanation)
    }

    /// "3f78754f · 2 of 5 jobs done · 12 min · Lander · Opus · 1.4M": the
    /// commit, how CI stands, how long it has been here and who lands it.
    /// The commit opens its run, the one place a landing goes.
    private var facts: some View {
        let words = Self.facts(card, ci: ci, now: now)
        return Group {
            if words.isEmpty {
                EmptyView()
            } else if let run {
                Button { openURL(run) } label: { line(words) }
                    .buttonStyle(.plain)
                    .help("Open CI on \(run.host() ?? "GitHub")")
            } else {
                line(words)
            }
        }
        .accessibilityLabel(card.accessibleFacts(ci: ci, now: now))
        .identified("plan-landing-\(landing.number)-words")
    }

    private func line(_ words: String) -> some View {
        Text(words)
            .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: card.phase.needsAttention ? .medium : .regular))
            .foregroundStyle(card.phase.needsAttention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
    }

    /// The facts line's words (AgentKit's), apart so a test reads them.
    static func facts(_ card: PlanSections.LandingCard, ci: PlanCIRead?, now: Int64) -> String { card.facts(ci: ci, now: now) }

    private var accessibility: String {
        let step = PlanLadder.step(card.phase)
        let place = step >= PlanLadder.steps.count ? "" : ", step \(step + 1) of \(PlanLadder.steps.count)"
        return [landing.label, landing.subtitle, landing.carries, card.phase.word + place].compactMap { $0 }.joined(separator: ", ")
    }
}

/// Where a landing stands, as three segments and the words under them, each
/// explained on hover. Roomy, the three steps are named; narrow, the phase
/// alone is.
struct PlanLadderView: View {
    let phase: LandingPhase
    @Environment(\.colorScheme) private var scheme

    private var current: Int { PlanLadder.step(phase) }
    private var tint: Color? { phase.needsAttention ? Tint.attention(scheme) : nil }

    var body: some View {
        ViewThatFits(in: .horizontal) {
            named
            compact
        }
        .accessibilityHidden(true)
        .identified("plan-ladder")
    }

    private func segment(_ index: Int) -> some View {
        Capsule()
            .fill(index < current ? AnyShapeStyle(.secondary) : index == current ? AnyShapeStyle(tint ?? .primary) : AnyShapeStyle(.quaternary))  // style-exempt: a progress mark, not text
            .frame(height: PlanMetrics.bar)
    }

    private var named: some View {
        HStack(alignment: .top, spacing: Spacing.group) {
            ForEach(0..<PlanLadder.steps.count, id: \.self) { index in
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    segment(index)
                    Text(PlanLadder.label(index, current: current, phase: phase))
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: index == current ? .medium : .regular))
                        .foregroundStyle(
                            index == current
                                ? AnyShapeStyle(tint ?? Color.primary) : AnyShapeStyle(.quietInk)
                        )
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                // Equal columns, so the ladders line up from card to card; a long phase wraps in its own.
                .frame(maxWidth: .infinity, alignment: .leading)
                .help(PlanLadder.explanation(index, current: current, phase: phase))
                .identified("plan-ladder-step-\(index)")
            }
        }
        .frame(minWidth: 330, maxWidth: .infinity, alignment: .leading)
    }

    private var compact: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            HStack(spacing: Spacing.tight) {
                ForEach(0..<PlanLadder.steps.count, id: \.self) { segment($0) }
            }
            Text(phase.word)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                .foregroundStyle(tint ?? Color.primary)
                .lineLimit(1)
        }
        .help(phase.explanation)
    }
}

/// One ticket inside a landing: its title and how it stands, and its key.
struct PlanLandingTicketRow: View {
    let ticket: PlanSections.LandingTicket
    let now: Int64
    let selected: Bool
    let keyed: Bool
    let onOpen: () -> Void
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    /// "Needs you" is the ticket's note, said once, on the right.
    private var warning: String? { ticket.waitsOnOwner ? nil : PlanWords.stale(ticket.line, now: now) }
    /// Over its token budget (ov-307), beside whatever else it says.
    private var overBudget: String? { PlanWords.overBudget(ticket.line) }
    private var amber: Bool { warning != nil || overBudget != nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button(action: onOpen) {
                HStack(alignment: .firstTextBaseline, spacing: 0) {
                    Image(systemName: glyph)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(amber ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
                        .glyphColumn()
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                            Text(ticket.line.heading)
                                .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                                .lineLimit(2)
                                .fixedSize(horizontal: false, vertical: true)
                                .layoutPriority(1)
                            Spacer(minLength: 0)
                            if let note = ticket.note {
                                Text(note)
                                    .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: ticket.noteNeedsAttention ? .medium : .regular))
                                    .foregroundStyle(ticket.noteNeedsAttention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
                                    .lineLimit(1)
                                    .fixedSize()
                                    .layoutPriority(2)
                                    // Needs you is the ticket's warning, said once, here: its id is the warning's.
                                    .probed(ticket.noteNeedsAttention ? "plan-ticket-\(ticket.key)-warning" : "plan-landing-ticket-\(ticket.key)-note")
                            }
                        }
                        if !ticket.line.title.isEmpty {
                            TaskKeyText(keysIn: ticket.key)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(.quietInk)
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                        ForEach([warning, overBudget].compactMap { $0 }, id: \.self) { line in
                            Text(line)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                                .foregroundStyle(Tint.attention(scheme))
                                .probed(line == overBudget ? "plan-ticket-\(ticket.key)-over-budget" : "plan-ticket-\(ticket.key)-warning")
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
            .accessibilityLabel([ticket.line.heading, ticket.note, warning, PlanWords.overBudgetSpoken(ticket.line)].compactMap { $0 }.joined(separator: ", "))
            .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
            .help(ticket.line.branch)
            .identified("plan-landing-ticket-\(ticket.key)")
        }
    }

    private var glyph: String {
        switch ticket.standing {
        // Simply aboard says nothing: its state's own mark, never a check that reads as "ready".
        case .ready: return PlanGlyph.name(ticket.line)
        case .fixing: return "wrench.and.screwdriver"
        case .landed: return "checkmark.circle.fill"
        case .notReady(let stage): return PlanGlyph.name(stage)
        }
    }
}
