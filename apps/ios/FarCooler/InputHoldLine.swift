import SwiftUI

/// The one quiet line over a terminal when typed input didn't get through
/// (ov-238). Held input offers Try Again and Discard; typing that may have
/// arrived offers neither, because sending it again could type it twice.
/// Not a red banner or an alert: the pane is fine and the person is
/// mid-keystroke, so it reads as a footnote with a button or two.
struct InputHoldLine: View {
    @ObservedObject var hold: InputHold
    let retry: () -> Void

    var body: some View {
        if let sentence = hold.sentence {
            HStack(spacing: 12) {
                Text(sentence)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if hold.isHolding {
                    Button(InputHold.retryTitle, action: retry)
                        .font(.footnote.weight(.semibold))
                        .accessibilityIdentifier("terminal-unsent-retry")
                    Button(InputHold.discardTitle) { hold.discard() }
                        .font(.footnote)
                        .accessibilityIdentifier("terminal-unsent-discard")
                } else {
                    Button(InputHold.dismissTitle) { hold.dismissMaybeLost() }
                        .font(.footnote.weight(.semibold))
                        .accessibilityIdentifier("terminal-unsent-dismiss")
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .background(.regularMaterial)  // style-exempt: frosts the unsent line over terminal text
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("terminal-unsent")
        }
    }
}
