import AgentKit
import SwiftUI

// A ticket's row in In Review, Building, Waiting and Waiting for the Next
// Landing (ov-503, design 2), drawn from the Rust core's card (`PlanView.
// TicketCard`, ov-1016): its title and the theme it serves, the second line
// (Next Up's reason, else its stage, key, agent, tokens and age), then its
// notes, one a line: the pull request's stage, the engine's step and gate,
// and its warnings, amber when they need a person. In Next Up its rank sits in
// the glyph column; elsewhere its stage's mark, amber when a note is.
//
//   ⚒  The themes grid                         Trustworthy agent view
//      Building · ov-310 · Builder · Sonnet · 380K · 1 h
//      Prepare Workspace · Approval Required

struct PlanCardRow: View {
    let card: PlanView.TicketCard
    let selected: Bool
    let keyed: Bool
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                mark.glyphColumn()
                VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                    // The theme sits beside the title when both fit on a line,
                    // else under it: a narrow column never squeezes either to
                    // an ellipsis.
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                            title.fixedSize(horizontal: true, vertical: false)
                            Spacer(minLength: 0)
                            themeName
                        }
                        VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                            title
                            themeName
                        }
                    }
                    // A key in the reason shows its card on hover (ov-299).
                    TaskKeyText(keysIn: card.second)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.quietInk)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                    ForEach(Array(card.notes.enumerated()), id: \.offset) { _, note in
                        PlanNoteLine(note: note, probe: Self.probe(note, key: card.key))
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
        .accessibilityLabel(card.accessibility)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .help(card.branch)
        .identified("plan-ticket-\(card.key)")
    }

    /// Each note's probe id: the stage's (amber or not), the run's, the
    /// warning's and the budget's.
    static func probe(_ note: PlanView.Note, key: String) -> String {
        switch note.kind {
        case .stage: note.attention ? "plan-ticket-\(key)-stage-attention" : "plan-ticket-\(key)-stage"
        case .run: "plan-ticket-\(key)-run"
        case .warning: "plan-ticket-\(key)-warning"
        case .overBudget: "plan-ticket-\(key)-over-budget"
        case .unknown: "plan-ticket-\(key)-note"
        }
    }

    private var title: some View {
        Text(card.heading)
            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
    }

    /// Which theme it serves (the owner's ask, ov-273).
    @ViewBuilder private var themeName: some View {
        if let theme = card.theme {
            Text(theme)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.quietInk)
                .lineLimit(1)
                .truncationMode(.tail)
                .probed("plan-ticket-\(card.key)-theme")
        }
    }

    @ViewBuilder private var mark: some View {
        if let rank = card.rank {
            Text("\(rank)")
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold).monospacedDigit())
                .foregroundStyle(.quietInk)
        } else {
            Image(systemName: PlanGlyph.name(card.mark))
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(card.markAttention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
                .accessibilityHidden(true)
        }
    }
}

/// One of a ticket's notes: gray, or amber and a little heavier when it needs
/// a person. A stage or a run note keeps to one line, elided in the middle (a
/// reviewer's login may be long, ov-316), and says every word on hover.
struct PlanNoteLine: View {
    let note: PlanView.Note
    let probe: String
    @Environment(\.colorScheme) private var scheme

    private var oneLine: Bool { note.kind == .stage || note.kind == .run }

    var body: some View {
        Text(note.text)
            .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: note.attention ? .medium : .regular))
            .foregroundStyle(note.attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.quietInk))
            .lineLimit(oneLine ? 1 : 2)
            .truncationMode(.middle)
            .help(note.help ?? "")
            .probed(probe)
    }
}

extension PlanGlyph {
    /// A card's mark, as the view names it.
    static func name(_ mark: PlanView.Mark) -> String {
        switch mark {
        case .rank, .unknown: "questionmark.circle"
        case .waiting: name(TicketStage.waiting)
        case .building: name(TicketStage.building)
        case .fixing: "wrench.and.screwdriver"
        case .reviewing: name(TicketStage.reviewing)
        case .needsYou: name(TicketStage.needsYou)
        case .landing: name(TicketStage.landing)
        case .landed: name(TicketStage.landed)
        case .dropped: name(TicketStage.dropped)
        }
    }
}
