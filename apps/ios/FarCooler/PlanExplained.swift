import SwiftUI

// A long press explains (ov-522): what a train is, or what each step of the
// ladder means, in lines a person can read in a breath. A popover on a phone,
// since there is no hover; the Mac says the same words as a tooltip.

private struct ExplainedOnLongPress: ViewModifier {
    let lines: [String]
    @State private var shown = false

    func body(content: Content) -> some View {
        content
            .onLongPressGesture(minimumDuration: 0.4) { shown = true }
            .popover(isPresented: $shown) {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
                        Text(line)
                            .font(index == 0 ? .subheadline : .footnote)
                            .foregroundStyle(index == 0 ? .primary : .secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding()
                .frame(maxWidth: 320, alignment: .leading)
                .presentationCompactAdaptation(.popover)
                .accessibilityIdentifier("plan-explanation")
            }
            .accessibilityAction(named: "Explain") { shown = true }
    }
}

extension View {
    /// A long press (and VoiceOver's "Explain") shows `lines` in a popover.
    func explainedOnLongPress(_ lines: [String]) -> some View {
        modifier(ExplainedOnLongPress(lines: lines))
    }
}
