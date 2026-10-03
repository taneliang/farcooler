import SwiftUI

/// The one full-pane status: an optional mark (a spinner or a thin symbol), a
/// headline, a sentence, the host's own words in a box, and one way out.
///
/// A terminal pane and an agent session each drew this by hand, with the same
/// proportions and drifting in small ways. The proportions are
/// `FleetView.failure`'s: a 42-point thin mark, a `.title2` headline, a
/// `.callout` sentence, 22 and 8 points between them. Screens that want the
/// system's own look use `ContentUnavailableView`; this is for a pane's state
/// on a ground the pane paints itself. Android draws the same shape with
/// `EmptyState`.
///
/// `message` is prose this app wrote; `transcript` is what the host said, and
/// the two are kept apart on purpose (see `TerminalView.status`'s history).
struct EmptyState: View {
    var spinner = false
    var symbol: String?
    var mark: Color = .secondary
    let title: String
    var message: String?
    var transcript: String?
    var actionTitle: String?
    var action: (() -> Void)?
    /// A second way out, beside the first: a lost terminal's Dismiss next to
    /// its Restart (ov-191). Nil everywhere else.
    var secondaryTitle: String?
    var secondary: (() -> Void)?
    /// For a UI test to read the headline and the sentence by name.
    var titleID: String?
    var messageID: String?

    var body: some View {
        VStack(spacing: 0) {
            if spinner {
                ProgressView()
                    .controlSize(.large)
                    .padding(.bottom, 22)
            } else if let symbol {
                Image(systemName: symbol)
                    .font(.system(size: 42, weight: .thin))
                    .foregroundStyle(mark)
                    .padding(.bottom, 22)
            }
            Text(title)
                .font(.title2.weight(.semibold))
                .multilineTextAlignment(.center)
                .padding(.bottom, 8)
                .accessibilityIdentifier(titleID ?? "empty-state-title")
            if let message {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 320)
                    .accessibilityIdentifier(messageID ?? "empty-state-message")
            }
            if let transcript, !transcript.isEmpty {
                DetailBox(text: transcript)
                    .frame(maxWidth: 320)
                    .padding(.top, 14)
            }
            // Bordered rather than prominent: a way out of a state that is not
            // an error, and an accented button would read as the app asking to
            // be tapped.
            if let actionTitle, let action {
                HStack(spacing: 12) {
                    Button(actionTitle, action: action)
                        .buttonStyle(.bordered)
                    if let secondaryTitle, let secondary {
                        Button(secondaryTitle, action: secondary)
                            .buttonStyle(.bordered)
                    }
                }
                .padding(.top, 22)
            }
        }
    }
}
