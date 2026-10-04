import SwiftUI

/// The one quiet line over a terminal when typed input didn't reach the runner.
///
/// It says why and offers Try Again, which sends what's held first and in order
/// (ov-238). Not a red banner or an alert: the pane is fine and the person is
/// mid-keystroke, so it reads as a footnote with one button.
struct UnsentInputLine: View {
    let unsent: UnsentInput
    let retry: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Text(unsent.sentence)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button(UnsentInput.retryTitle, action: retry)
                .font(.footnote.weight(.semibold))
                .accessibilityIdentifier("terminal-unsent-retry")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.regularMaterial)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("terminal-unsent")
    }
}
