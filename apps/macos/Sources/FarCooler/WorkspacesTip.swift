import SwiftUI

/// The one-time tip over the sidebar: the orchestrator fills the workspace,
/// with the board on the right (ov-89). It said once that workspaces had
/// become places in the sidebar (spec §9), under `tips.workspaces`, then that
/// tasks opened beside the board (ov-85), under `tips.tasksBesideBoard`; the
/// layouts those described are gone, so this one has a key of its own, and
/// shows once even to whoever dismissed them.
enum WorkspacesTip {
    /// Set once the tip has been dismissed; never cleared.
    static let key = "tips.orchestratorFillsWorkspace"

    static let title = "The orchestrator now fills the workspace."
    static let message =
        "Your board is on the right. Click a task to open it here, press ↑ or ↓ to look through the others, and Esc to go back to the orchestrator."

    static func shouldShow(_ defaults: UserDefaults = .standard) -> Bool {
        !defaults.bool(forKey: key)
    }

    static func dismiss(_ defaults: UserDefaults = .standard) {
        defaults.set(true, forKey: key)
    }
}

/// The tip itself: two sentences and OK.
struct WorkspacesTipView: View {
    var onDismiss: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(WorkspacesTip.title).font(.callout.weight(.semibold))
            Text(WorkspacesTip.message)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                // No default-action key: Return belongs to the terminal
                // beside it.
                Button("OK", action: onDismiss)
                    .controlSize(.small)
            }
        }
        .padding(12)
        .floatingPanel()
        .padding(10)
        .accessibilityElement(children: .contain)
    }
}
