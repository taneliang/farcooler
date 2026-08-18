import AppKit
import SwiftUI

/// Remote Login, which macOS keeps off and Far Cooler cannot turn on.
///
/// A phone reaches a Mac over SSH — there is no Far Cooler listener anywhere,
/// which is the architecture and not an omission — and a Mac ships with sshd
/// disabled. So a Mac granted as a runner is reachable by nothing until a
/// person allows it, and the honest thing is to say so and open the pane.
///
/// **It does not claim to know whether Remote Login is on.** Asking properly
/// means `systemsetup -getremotelogin`, which needs root; the alternatives are
/// probing port 22 on this Mac, which a firewall answers for, or reading a
/// launchd label Apple has moved before. A screen that says "Remote Login is
/// off" when it is on is worse than one that simply says what to check.
struct RemoteLoginView: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Turn on Remote Login").font(.headline)
            Text("Far Cooler uses SSH to connect to this Mac from your other devices.")
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            Text("Open System Settings › General › Sharing and turn on Remote Login.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            Button("Open Sharing Settings", action: openSharingSettings)
        }
    }

    /// The Sharing pane, by the URL System Settings registers for it.
    ///
    /// `x-apple.systempreferences:` with an extension identifier, rather than
    /// opening `System Settings.app` and leaving somebody to find Sharing. A
    /// Mac where the URL does not resolve — an older System Settings, a managed
    /// install — opens nothing, which is why the sentence above names the path
    /// as well as the button offering it.
    private func openSharingSettings() {
        guard
            let url = URL(
                string: "x-apple.systempreferences:com.apple.Sharing-Settings.extension")
        else { return }
        NSWorkspace.shared.open(url)
    }
}
