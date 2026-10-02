import SwiftUI

/// The one-time tip over the sidebar on the first launch after workspaces
/// became places (spec §9).
enum WorkspacesTip {
    /// Set once the tip has been dismissed; never cleared.
    static let key = "tips.workspaces"

    static let title = "Workspaces are now in the sidebar."
    static let message = "Select one to see its board, with its orchestrator on the rail beside it. Its worktrees are one click down."

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
