import SwiftUI

/// A draft held behind a dialog in this pane (ov-385): "Waiting for the
/// dialog to close" with Withdraw while it waits, then "Sent", or why not.
///
/// Over the orchestrator's terminal, where Ask the Orchestrator, Reverse and
/// Discuss go after the runner holds their draft, and where the dialog it
/// waits on is. Read from the pane's `draftHold`, so a draft another device
/// sent shows here too. The words are `HeldDraft`'s, in AgentKit.
struct HeldDraftBar: View {
    @ObservedObject var connection: Connection
    let terminal: Terminal
    @State private var watch = HeldDraft.Watch()
    @State private var withdrawing = false

    var body: some View {
        VStack(spacing: 0) {
            if let status = watch.status(terminal.draftHold), let title = HeldDraft.title(status) {
                bar(status, title: title)
            }
        }
        .onAppear { watch.observe(terminal.draftHold) }
        .onChange(of: terminal.draftHold) { _, hold in
            watch.observe(hold)
            // A newer hold, or this one ended: Withdraw is pressable again.
            withdrawing = false
        }
    }

    private func bar(_ status: HeldDraft.Status, title: String) -> some View {
        HStack(alignment: .center, spacing: 12) {
            if status == .waiting {
                ProgressView()
            } else {
                Image(systemName: status == .sent ? "checkmark.circle" : "exclamationmark.circle")
                    .foregroundStyle(.secondary)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.subheadline.weight(.semibold))
                if let detail = HeldDraft.detail(status) {
                    Text(detail).font(.footnote).foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if status == .waiting, let hold = terminal.draftHold {
                Button(HeldDraft.withdraw) { withdraw(hold) }
                    .disabled(withdrawing)
                    .accessibilityIdentifier("held-draft-withdraw")
            } else {
                Button { watch.dismiss() } label: { Image(systemName: "xmark") }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Dismiss")
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar) // style-exempt: the system bar material, as the pane's other bars use
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("held-draft")
    }

    private func withdraw(_ hold: DraftHold) {
        withdrawing = true
        Task { @MainActor in
            // Not reached: the pane still waits, and Withdraw is there again.
            if !(await connection.withdrawDraft(terminal: terminal.id, hold: hold.id)) { withdrawing = false }
        }
    }
}

extension Connection {
    /// Stop the runner pasting a held draft. The pane's next read says how it
    /// ended. False when it didn't reach the runner: it still waits.
    func withdrawDraft(terminal: String, hold: String) async -> Bool {
        let reached: Bool
        do {
            _ = try await rpc("terminal.draft_withdraw", ["terminal": terminal, "hold": hold])
            reached = true
        } catch {
            reached = false
        }
        await refresh()
        return reached
    }
}
