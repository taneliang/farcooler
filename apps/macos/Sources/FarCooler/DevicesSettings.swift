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
/// method and the client core's dispatch now routes it, but the CLI — which is
/// how a Mac reaches a runner — still has no `client` subcommand, which is the
/// gap `Enrollment` names. When the rows arrive, a Mac is TWO of them: one line
/// per key, told apart by `shellAccess`, removed together.
struct DevicesSettings: View {
    @State private var adding = false

    var body: some View {
        Form {
            Section {
                // One button, where there were two — and the two were opposite
                // directions of one ceremony, named "Add Device…" and "Add This
                // Mac to Another Account…", which share no words and so read as
                // unrelated features. Both were greyed out when signed out,
                // with the reason in a footer directing you to a third tab.
                //
                // `AddView` carries both directions, says which is which, and
                // signs you in where you are rather than sending you away. It
                // also carries the third thing that was never here at all:
                // adding a runner, which lived as a bare text field under
                // Runners and is the answer when there is no other device to
                // scan.
                Button("Add…") { adding = true }
            } header: {
                Text("Devices")
            } footer: {
                Text(
                    "New devices can access only the runners you select. Runners you add "
                        + "later aren’t shared automatically."
                )
                .fixedSize(horizontal: false, vertical: true)
            }

            Section {
                RemoteLoginView()
            }
        }
        .formStyle(.grouped)
        .sheet(isPresented: $adding) { AddView() }
    }
}
