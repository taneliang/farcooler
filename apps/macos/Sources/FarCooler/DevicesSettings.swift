import AgentKit
import SwiftUI

/// Settings › Devices — the two directions of the ceremony, and the one thing
/// macOS will not let this app do for you.
///
/// Its own tab rather than a section under Runners, because the copy everywhere
/// else in this product says "Settings › Devices" and a screen that is not
/// where it says it is costs somebody a minute every time.
///
/// **Access is derived, never stored.** What each device may reach is the
/// runner's own answer to `client.list`, read on every look — no record here
/// can disagree with `~/.ssh/authorized_keys`, because the file is the one of
/// the two that decides. That list is not on screen yet: the daemon serves the
/// method and neither the CLI nor the client core's dispatch offers a route to
/// it, which is the gap `Enrollment` names.
struct DevicesSettings: View {
    @ObservedObject private var account = Account.shared
    @State private var adding = false
    @State private var joining = false

    var body: some View {
        Form {
            Section {
                Button("Add Device…") { adding = true }
                    .disabled(!account.isSignedIn)
                Button("Add This Mac to Another Account…") { joining = true }
                    .disabled(!account.isSignedIn)
            } header: {
                Text("Devices")
            } footer: {
                Text(
                    account.isSignedIn
                        ? "Adding a device grants it the runners you pick, one at a time. "
                            + "A runner you add later grants nothing on its own."
                        : "Sign in first — Settings › Account."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            }

            Section {
                RemoteLoginView()
            }
        }
        .formStyle(.grouped)
        .sheet(isPresented: $adding) { AddDeviceView() }
        .sheet(isPresented: $joining) { JoinView() }
    }
}
