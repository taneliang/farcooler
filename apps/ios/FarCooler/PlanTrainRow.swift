import SwiftUI

// A train heading its lanes in Now (ov-309), on the iPhone as on the Mac: its
// name, where it stands and its CI as the runner last read it. Red, or a
// failed run, is the one thing drawn in amber, with its word. A tap opens the
// CI run in the browser, the one place it goes.

struct PlanTrainRow: View {
    let train: PlanTrain
    let ci: PlanCIRead?
    @Environment(\.colorScheme) private var scheme
    @Environment(\.openURL) private var openURL

    private var words: String { PlanWords.train(train, ci: ci) }
    private var attention: Bool { PlanWords.trainNeedsAttention(train, ci: ci) }
    private var run: URL? { ci.flatMap { PageLinks.https($0.url) } }

    var body: some View {
        Button {
            if let run { openURL(run) }
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.card) {
                Image(systemName: "train.side.front.car")
                    .foregroundStyle(attention ? AnyShapeStyle(GlancePalette.amber(scheme)) : AnyShapeStyle(.secondary))
                    .frame(width: 22, alignment: .leading)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(train.name).font(.body.weight(.semibold))
                    Text(words)
                        .font(.footnote.weight(attention ? .medium : .regular))
                        .foregroundStyle(attention ? AnyShapeStyle(GlancePalette.amber(scheme)) : AnyShapeStyle(.secondary))
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
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
        .buttonStyle(.plain)
        .disabled(run == nil)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Train \(train.name), \(words)")
        .accessibilityAddTraits(run == nil ? [] : .isLink)
        .accessibilityIdentifier("plan-train-\(train.name)")
    }
}
