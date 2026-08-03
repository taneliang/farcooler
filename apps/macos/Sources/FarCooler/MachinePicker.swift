import SwiftUI

/// Which machine you are looking at, and how to look at another one.
///
/// This heads the sidebar because the sidebar is that machine's workspaces —
/// and before this, it said "Fleet" whether you were driving this Mac or a
/// server in another country. A window that cannot tell you whose terminals are
/// in it is a window you can act on the wrong machine from.
///
/// The switcher lives here rather than in Settings for the same reason. Adding a
/// machine is configuration and belongs in Settings; *driving* one is a thing
/// you do several times an hour, and it was reachable only through a settings
/// pane nobody had a reason to open twice.
struct MachinePicker: View {
    @ObservedObject private var hosts = Hosts.shared
    @ObservedObject private var preferences = Preferences.shared

    var body: some View {
        Menu {
            Button {
                hosts.active = ""
            } label: {
                Label("This Mac", systemImage: "laptopcomputer")
            }

            if !hosts.all.isEmpty {
                Section("Machines") {
                    ForEach(hosts.all) { host in
                        Button {
                            hosts.active = host.target
                        } label: {
                            Label(host.target, systemImage: icon(for: host))
                        }
                        // A machine with nothing installed on it has no daemon
                        // to drive, so offering it would be offering an error.
                        // It is still listed, because "the one I added is greyed
                        // out" sends you to Settings, and "it is missing" sends
                        // you to add it a second time.
                        .disabled(host.probe?.isInstalled != true)
                    }
                }
            }

            Divider()
            // Both go to the same pane. Two verbs because someone with no
            // machines is looking for the first, and someone with three is
            // looking for the second.
            SettingsLink { Text(hosts.all.isEmpty ? "Add a machine…" : "Manage machines…") }
                .simultaneousGesture(TapGesture().onEnded { preferences.settingsTab = "machines" })
        } label: {
            // The compensation lives INSIDE the label, not on the `Menu`.
            // A borderless menu stops honouring negative padding on itself past
            // a few points — 20 and 29 rendered identically — so the only way
            // to reach the sidebar's edge is to move the label within the box
            // the menu gives it.
            HStack(spacing: 5) {
                Image(systemName: currentIcon)
                    .font(.system(size: 11))
                Text(currentName)
                    .font(.headline)
                    .lineLimit(1)
                // Drawn rather than left to `menuIndicator`, which sits too far
                // from the text and at the wrong weight for a title.
                Image(systemName: "chevron.down")
                    .font(.system(size: 8, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .padding(.leading, 1)
            }
            .padding(.leading, -Grid.menuChromeLeading)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        // A borderless menu insets its own label, so the icon landed several
        // points right of the section headers below it and the whole band read
        // as misaligned. The header pads to the same rail as everything else
        // and this takes the chrome back out.
        // Named in full here even though the label truncates: an ssh alias and
        // the machine it points at are not always the same word.
        .help(hosts.active.isEmpty ? "Driving this Mac" : "Driving \(hosts.active)")
    }

    private var currentName: String {
        hosts.active.isEmpty ? "This Mac" : hosts.active
    }

    private var currentIcon: String {
        guard !hosts.active.isEmpty else { return "laptopcomputer" }
        guard let host = hosts.all.first(where: { $0.target == hosts.active }) else {
            return "server.rack"
        }
        return icon(for: host)
    }

    private func icon(for host: RemoteHost) -> String {
        switch host.probe?.platform {
        case "macos": return "desktopcomputer"
        case "wsl": return "pc"
        case "linux": return "server.rack"
        default: return "network"
        }
    }
}
