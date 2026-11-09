import AgentKit
import SwiftUI

// A landing in Landing (ov-503, design 2; ov-522): a card headed "Landing 86"
// with its title under it and what it carries, a ladder of where it stands in
// three plain steps (each explained on hover), the CI line, and its tickets. A
// ticket aboard says nothing; it speaks only once landed or when it waits on
// the owner. Amber only for CI that failed, a landing that stalled, and a
// ticket waiting on the owner, each with words. Every word, the ladder and the
// tickets' standing are the Rust core's (`WireLandingCard`, ov-1016); the
// CI read beside the commit is the app's (ov-1015 phase 2).
//
//   Landing 86
//   The team directory; the iPhone composer
//   ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬ ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬ ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬
//   Checking Here    Checking on CI  Landed
//   3f78754f · 2 of 5 jobs done · 12 min · Lander · Opus · 1.4M
//   ✓ team-dir
//   ios-overwrite                                   Needs you

struct PlanLandingCard: View {
    let card: WireLandingCard
    /// What a landing is, said on hover over its heading: the Landing section's help.
    let explanation: String?
    let ci: PlanCIRead?
    let now: Int64
    let selected: PlanPage?
    let keyed: Bool
    let onOpen: (PlanPage) -> Void
    @Environment(\.colorScheme) private var scheme
    @Environment(\.openURL) private var openURL

    private var run: URL? { ci.flatMap { PageLinks.https($0.url) } }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            heading
            PlanLadderView(ladder: card.ladder, word: card.phaseWord, explanation: card.explanation, attention: card.attention)
            facts
            if !card.tickets.isEmpty {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(card.tickets) { ticket in
                        PlanLandingTicketRow(
                            ticket: ticket,
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
        .identified("plan-landing-\(card.number)")
    }

    private var heading: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
            Text(card.label)
                .font(.system(size: WorkspaceStyle.PaneText.title, weight: .semibold))
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
                .identified("plan-landing-\(card.number)-label")
            if let subtitle = card.subtitle {
                Text(subtitle)
                    .font(.system(size: WorkspaceStyle.PaneText.body))
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .identified("plan-landing-\(card.number)-subtitle")
            }
            if let carries = card.carries {
                Text(carries)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.quietInk)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .identified("plan-landing-\(card.number)-carries")
            }
        }
        // What a landing is, said here and nowhere in the text.
        .help(explanation ?? "")
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
        .identified("plan-landing-\(card.number)-words")
    }

    private func line(_ words: String) -> some View {
        Text(words)
            .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: card.attention ? .medium : .regular))
            .foregroundStyle(card.attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
    }

    /// The facts line's words (AgentKit's), apart so a test reads them.
    static func facts(_ card: WireLandingCard, ci: PlanCIRead?, now: Int64) -> String { card.facts(ci: ci, now: now) }

    private var accessibility: String {
        [card.label, card.subtitle, card.carries, card.ladder.compact].compactMap { $0 }.joined(separator: ", ")
    }
}

/// Where a landing stands, as three segments and the words under them, each
/// explained on hover. Roomy, the three steps are named; narrow, the phase
/// alone is.
struct PlanLadderView: View {
    let ladder: WireLadder
    /// The phase alone, for the compact form.
    let word: String
    let explanation: String
    let attention: Bool
    @Environment(\.colorScheme) private var scheme

    private var current: Int { Int(ladder.at) }
    private var tint: Color? { attention ? Tint.attention(scheme) : nil }

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
            ForEach(Array(ladder.steps.enumerated()), id: \.offset) { index, step in
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    segment(index)
                    Text(step.label)
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
                .help(step.explanation)
                .identified("plan-ladder-step-\(index)")
            }
        }
        .frame(minWidth: 330, maxWidth: .infinity, alignment: .leading)
    }

    private var compact: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            HStack(spacing: Spacing.tight) {
                ForEach(ladder.steps.indices, id: \.self) { segment($0) }
            }
            Text(word)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                .foregroundStyle(tint ?? Color.primary)
                .lineLimit(1)
        }
        .help(explanation)
    }
}

/// One ticket inside a landing: its title and how it stands, and its key.
struct PlanLandingTicketRow: View {
    let ticket: WireTicketCard
    let selected: Bool
    let keyed: Bool
    let onOpen: () -> Void
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    /// Under its title: its notes, less the "Needs you" it leads with while it
    /// waits on the owner, which is drawn beside the title.
    private var warnings: ArraySlice<WireNote> { ticket.notes.dropFirst(ticket.waitsOnOwner ? 1 : 0) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button(action: onOpen) {
                HStack(alignment: .firstTextBaseline, spacing: 0) {
                    Image(systemName: glyph)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(!warnings.isEmpty ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
                        .glyphColumn()
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                            Text(ticket.heading)
                                .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                                .lineLimit(2)
                                .fixedSize(horizontal: false, vertical: true)
                                .layoutPriority(1)
                            Spacer(minLength: 0)
                            if let note = ticket.note {
                                Text(note)
                                    .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: ticket.waitsOnOwner ? .medium : .regular))
                                    .foregroundStyle(ticket.waitsOnOwner ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
                                    .lineLimit(1)
                                    .fixedSize()
                                    .layoutPriority(2)
                                    // Needs you is the ticket's warning, said once, here: its id is the warning's.
                                    .probed(ticket.waitsOnOwner ? "plan-ticket-\(ticket.key)-warning" : "plan-landing-ticket-\(ticket.key)-note")
                            }
                        }
                        // The key under a title; a ticket with no title is its key already.
                        if ticket.heading != ticket.key {
                            TaskKeyText(keysIn: ticket.key)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(.quietInk)
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                        ForEach(Array(warnings.enumerated()), id: \.offset) { _, note in
                            PlanNoteLine(note: note, probe: note.probe(key: ticket.key))
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
            .accessibilityLabel(ticket.accessibility)
            .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
            .help(ticket.branch)
            .identified("plan-landing-ticket-\(ticket.key)")
        }
    }

    /// Simply aboard says nothing: its state's own mark, never a check that
    /// reads as "ready"; landed, a filled check.
    private var glyph: String {
        ticket.mark == .landed ? "checkmark.circle.fill" : PlanGlyph.name(ticket.mark)
    }
}
