import AgentKit
import SwiftUI

/// A draft held behind a dialog in this pane (ov-385): "Waiting for the
/// dialog to close" with Withdraw while it waits, then "Sent", or why not.
///
/// Over the orchestrator's terminal, where Ask the Orchestrator, Reverse and
/// Discuss leave the keyboard after the runner holds their draft, and where
/// the dialog it waits on is. Read from the pane's `draftHold`, which every
/// terminal event carries, so a draft a phone sent shows here too. The words
/// are `HeldDraft`'s, in AgentKit, which the iPhone says too.
struct HeldDraftBar: View {
    let hold: DraftHold?
    let withdraw: () -> Void
    @State private var watch = HeldDraft.Watch()
    @State private var withdrawing = false

    var body: some View {
        VStack(spacing: 0) {
            if let status = watch.status(hold), let title = HeldDraft.title(status) {
                bar(status, title: title)
            }
        }
        .onAppear { watch.observe(hold) }
        .onChange(of: hold) { _, hold in
            watch.observe(hold)
            withdrawing = false
        }
    }

    private func bar(_ status: HeldDraft.Status, title: String) -> some View {
        HStack(alignment: .center, spacing: Spacing.inset) {
            if status == .waiting {
                ProgressView().controlSize(.small)
            } else {
                Image(systemName: status == .sent ? "checkmark.circle" : "exclamationmark.circle")
                    .foregroundStyle(.secondary)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.callout.weight(.semibold))
                if let detail = HeldDraft.detail(status) {
                    Text(detail).font(.callout).foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if status == .waiting {
                Button(HeldDraft.withdraw) {
                    withdrawing = true
                    withdraw()
                }
                .disabled(withdrawing)
                .accessibilityIdentifier("held-draft-withdraw")
            } else {
                Button { watch.dismiss() } label: { Image(systemName: "xmark") }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Dismiss")
            }
        }
        .padding(.horizontal, Spacing.inset)
        .padding(.vertical, Spacing.group)
        .surface(.inset, in: .control)
        .padding(Spacing.group)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("held-draft")
    }
}
