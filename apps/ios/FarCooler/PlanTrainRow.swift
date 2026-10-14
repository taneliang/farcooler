import SwiftUI

// A train in Landing (ov-309, ov-503, ov-522), on the iPhone as on the Mac:
// "Train 72" with its title under it and what it carries (ov-462), where it
// stands on the ladder as "Checking on CI · step 2 of 3" over three short
// segments (a long press explains the train and each step), the
// commit, how CI stands and how long it has waited, and who integrates. Amber
// only for CI that failed or a train that stalled or is held, each with its
// words. The arrow at the trailing edge opens the CI run in the browser, the
// one place it goes; a long press never does.
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
    /// The same words with each span held together: a line breaks between
    /// spans, never inside "12 min" (ov-515).
    private var factsShown: String { card.factsUnbroken(ci: ci, now: now) }

    var body: some View {
        // The row is not a button: a long press explains it, and a Button
        // fires on release however long it was held, which opened CI over the
        // explanation. The run opens from its own control, at the trailing edge.
        content
            .frame(maxWidth: .infinity, alignment: .leading)
            .modifier(TrainRowSpeech(spoken: card.spoken(ci: ci, now: now), id: train.name))
            .explainedOnLongPress(card.explanation)
            // On the title's line, outside the row's one spoken element.
            .overlay(alignment: .topTrailing) { runControl }
    }

    /// The CI run's own control, on the title's line: a long press on the row
    /// never opens it, since a Button fires on release however long it was held.
    @ViewBuilder private var runControl: some View {
        if let run {
            Button { openURL(run) } label: {
                Image(systemName: "arrow.up.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 44, height: 24)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Open CI Run")
            .accessibilityIdentifier("plan-train-\(train.name)-run")
        }
    }

    private var content: some View {
            HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.card) {
                Image(systemName: "train.side.front.car")
                    .foregroundStyle(attention ? AnyShapeStyle(GlancePalette.amber(scheme)) : AnyShapeStyle(.secondary))
                    .frame(width: 22, alignment: .leading)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 3) {
                    Text(train.label).font(.body.weight(.semibold))
                        .accessibilityIdentifier("plan-train-\(train.name)-label")
                    if let subtitle = train.subtitle {
                        Text(subtitle)
                            .font(.subheadline)
                            .lineLimit(3)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("plan-train-\(train.name)-subtitle")
                    }
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
                        Text(factsShown)
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
            }
            .contentShape(.rect)
    }
}

/// What VoiceOver reads for a train, and its identifier.
private struct TrainRowSpeech: ViewModifier {
    let spoken: String
    let id: String

    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .combine)
            .accessibilityLabel(spoken)
            .accessibilityIdentifier("plan-train-\(id)")
    }
}

/// Where a train stands, as three short segments: the ones climbed are
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
