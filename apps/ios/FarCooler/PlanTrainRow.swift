import SwiftUI

// A train in Shipping (ov-309, ov-503), on the iPhone as on the Mac: "Train
// 72" and what it carries (ov-462), its slug second, where it stands on the
// ladder as "Waiting for CI · step 4 of 5" over five short segments, the
// commit, how CI stands and how long it has waited, and who integrates. Amber
// only for CI that failed or a train that stalled or is held, each with its
// words. A tap opens the CI run in the browser, the one place it goes.
//
// The words are AgentKit's (`PlanSections.TrainCard`, `PlanLadder`); this
// draws them.

struct PlanTrainRow: View {
    let card: PlanSections.TrainCard
    let ci: PlanCIRead?
    /// The runner's clock, for how old a stale read is.
    var now: Int64 = 0
    @Environment(\.colorScheme) private var scheme
    @Environment(\.openURL) private var openURL

    private var train: PlanTrain { card.train }
    private var attention: Bool { card.phase.needsAttention || PlanWords.trainNeedsAttention(train, ci: ci) }
    private var run: URL? { ci.flatMap { PageLinks.https($0.url) } }
    private var facts: String { card.facts(ci: ci, now: now) }

    var body: some View {
        // A button only with a run to open: disabled, the whole row went grey.
        if let run {
            Button { openURL(run) } label: { content }
                .buttonStyle(.plain)
                .accessibilityAddTraits(.isLink)
                .modifier(TrainRowSpeech(card: card, facts: facts, id: train.name))
        } else {
            content.modifier(TrainRowSpeech(card: card, facts: facts, id: train.name))
        }
    }

    private var content: some View {
            HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.card) {
                Image(systemName: "train.side.front.car")
                    .foregroundStyle(attention ? AnyShapeStyle(GlancePalette.amber(scheme)) : AnyShapeStyle(.secondary))
                    .frame(width: 22, alignment: .leading)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 3) {
                    Text(train.heading).font(.body.weight(.semibold))
                    if let carries = train.carries {
                        Text(carries)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("plan-train-\(train.name)-carries")
                    }
                    Text(PlanLadder.compact(card.phase))
                        .font(.footnote.weight(attention ? .medium : .regular))
                        .foregroundStyle(attention ? AnyShapeStyle(GlancePalette.amber(scheme)) : AnyShapeStyle(.secondary))
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("plan-train-\(train.name)-phase")
                    PlanLadderBar(phase: card.phase, attention: attention)
                        .padding(.vertical, 2)
                    if !facts.isEmpty {
                        Text(facts)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .lineLimit(3)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("plan-train-\(train.name)-words")
                    }
                    if let held = card.held {
                        Text(held)
                            .font(.footnote.weight(.medium))
                            .foregroundStyle(GlancePalette.amber(scheme))
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("plan-train-\(train.name)-held")
                    }
                }
                Spacer(minLength: 0)
                if run != nil {
                    Image(systemName: "arrow.up.forward")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.tertiary)
                        .accessibilityHidden(true)
                }
            }
            .contentShape(.rect)
    }
}

/// What VoiceOver reads for a train, and its identifier.
private struct TrainRowSpeech: ViewModifier {
    let card: PlanSections.TrainCard
    let facts: String
    let id: String

    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .combine)
            .accessibilityLabel(
                [card.train.heading, card.train.carries, PlanLadder.compact(card.phase), facts, card.held]
                    .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: ", "))
            .accessibilityIdentifier("plan-train-\(id)")
    }
}

/// Where a train stands, as five short segments: the ones climbed are
/// filled, the one it is on is dark, or amber when it needs attention.
struct PlanLadderBar: View {
    let phase: TrainPhase
    let attention: Bool
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let current = PlanLadder.step(phase)
        HStack(spacing: 3) {
            ForEach(0..<PlanLadder.steps.count, id: \.self) { index in
                Capsule()
                    .fill(style(index, current: current))
                    .frame(height: 4)
            }
        }
        .accessibilityHidden(true)
    }

    private func style(_ index: Int, current: Int) -> AnyShapeStyle {
        if index < current { return AnyShapeStyle(.secondary) }
        if index == current { return attention ? AnyShapeStyle(GlancePalette.amber(scheme)) : AnyShapeStyle(.primary) }
        return AnyShapeStyle(.quaternary)
    }
}
