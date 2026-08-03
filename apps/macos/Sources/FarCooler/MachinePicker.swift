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
        // A `SidebarMenuButton`-style Button rather than a `Menu`: a borderless
        // menu positions its own label, and no padding here could bring it out
        // to the same edge as the search field directly below. See
        // `SidebarLayout.swift`.
        Button { present() } label: {
            HStack(spacing: 5) {
                Image(systemName: currentIcon)
                    .font(.system(size: 11))
                Text(currentName)
                    .font(.headline)
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(.system(size: 8, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .padding(.leading, 1)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        // Named in full even though the label truncates: an ssh alias and the
        // machine it points at are not always the same word.
        .help(hosts.active.isEmpty ? "Driving this Mac" : "Driving \(hosts.active)")
    }

    private func present() {
        let menu = NSMenu()

        let local = NSMenuItem(title: "This Mac", action: #selector(Invoker.fire), keyEquivalent: "")
        let localInvoker = Invoker { hosts.active = "" }
        local.target = localInvoker
        local.representedObject = localInvoker
        local.state = hosts.active.isEmpty ? .on : .off
        menu.addItem(local)

        if !hosts.all.isEmpty {
            menu.addItem(.separator())
            for host in hosts.all {
                let item = NSMenuItem(
                    title: host.target, action: #selector(Invoker.fire), keyEquivalent: "")
                let invoker = Invoker { hosts.active = host.target }
                item.target = invoker
                item.representedObject = invoker
                item.state = hosts.active == host.target ? .on : .off
                // A machine with nothing installed has no daemon to drive, so
                // offering it would be offering an error. Still listed, because
                // "the one I added is missing" sends you to add it twice.
                item.isEnabled = host.probe?.isInstalled == true
                menu.addItem(item)
            }
        }

        menu.addItem(.separator())
        // Two verbs because someone with no machines is looking for the first
        // and someone with three is looking for the second.
        let manage = NSMenuItem(
            title: hosts.all.isEmpty ? "Add a machine…" : "Manage machines…",
            action: #selector(Invoker.fire), keyEquivalent: "")
        let manageInvoker = Invoker {
            preferences.settingsTab = "machines"
            NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
        }
        manage.target = manageInvoker
        manage.representedObject = manageInvoker
        menu.addItem(manage)

        menu.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
    }

    private final class Invoker: NSObject {
        private let action: () -> Void
        init(_ action: @escaping () -> Void) { self.action = action }
        @objc func fire() { action() }
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
