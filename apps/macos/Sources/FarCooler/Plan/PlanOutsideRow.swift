import AgentKit
import SwiftUI

/// The footer under the themes (ov-331, design 2.2): what the week did outside
/// every theme, and the cards the orchestrator's housekeeping would tidy.
/// One quiet row, since this is a signal about the section and not a theme.
/// "11 cards to tidy" opens the list, each key with its hovercard.
struct PlanOutsideRow: View {
    let outside: PlanOutside
    let onOpen: (PlanPage) -> Void
    @State private var listing = false

    var body: some View {
        if !outside.isEmpty {
            VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                if let words = PlanWords.outside(outside) {
                    Text(words)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .probed("plan-outside")
                }
                if !outside.tidy.isEmpty {
                    Button(PlanWords.tidy(outside.tidy.count)) { listing = true }
                        .buttonStyle(.link)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .popover(isPresented: $listing, arrowEdge: .bottom) { tidyList }
                        .probed("plan-outside-tidy")
                }
            }
            .padding(.leading, NavigatorGrid.textInset)
            .padding(.top, Spacing.group)
            .padding(.bottom, NavigatorRhythm.air)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .contain)
            .identified("plan-outside-themes")
        }
    }

    /// Each card the CLI's "Worth a look" names, with why.
    private var tidyList: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            ForEach(outside.tidy, id: \.task) { card in
                TaskKeyText(keysIn: "\(card.key) · \(card.status.replacingOccurrences(of: "_", with: " ").capitalized)")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
            }
        }
        .padding(Spacing.inset)
    }
}
