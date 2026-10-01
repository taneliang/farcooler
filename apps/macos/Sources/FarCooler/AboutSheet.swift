import AgentKit
import SwiftUI

/// About Far Cooler — which build this is, and which one this Mac is running.
///
/// Replaces the standard panel rather than decorating it. That panel shows
/// `CFBundleShortVersionString` and `CFBundleVersion`, which for a beta and the
/// release it names are identical — so the window whose entire job is answering
/// "what am I running" could not. This one names the channel, and the daemon
/// version, because those two are what have to match.
///
/// This Mac only, deliberately, now that the app can be looking at several
/// runners at once: a fleet-wide roundup here would either duplicate
/// Settings ▸ Runners (which already shows each runner's installed version
/// next to its name) or race it, and "what am I running" — the question this
/// window answers — is a question about the app in your hand, which runs on
/// exactly one Mac regardless of how many runners it is talking to.
///
/// It is a window of its own, the way a Mac shows it: it opens
/// from the menu whether or not a main window exists, blocks nothing, and has
/// no button, because Esc and ⌘W close it.
struct AboutView: View {
    @Environment(\.dismissWindow) private var dismissWindow

    var body: some View {
        AboutContent()
            .padding(.bottom, 8)
            .background(
                // Esc, with no visible button to carry it.
                Button("Close") { dismissWindow(id: AboutView.windowID) }
                    .keyboardShortcut(.cancelAction)
                    .opacity(0)
                    .accessibilityHidden(true))
    }

    /// The scene's id, which the menu item opens by.
    static let windowID = "about"
}

/// What About says, with no chrome around it.
private struct AboutContent: View {
    @State private var daemon: DaemonBuild?

    var body: some View {
        VStack(spacing: 0) {
            VStack(spacing: 10) {
                if let icon = NSApp.applicationIconImage {
                    Image(nsImage: icon)
                        .resizable()
                        .frame(width: 72, height: 72)
                }
                Text("Far Cooler").font(.title2.weight(.semibold))
                Text(AppVersion.display)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            .padding(.top, 24)
            .padding(.bottom, 18)

            Form {
                // Named explicitly rather than left at `VersionSection`'s
                // default empty host: the default reads as "no runner to
                // say," which was right when a blank host meant "whichever
                // one is being driven" and is wrong now that this window only
                // ever shows this Mac's own daemon.
                VersionSection(daemon: daemon, host: "This Mac") { text in
                    // The moment this window matters is when someone is filling
                    // in a bug report, and a commit hash transcribed by hand is
                    // a commit hash typed wrong.
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(text, forType: .string)
                }
            }
            .formStyle(.grouped)
            .scrollDisabled(true)
        }
        .frame(width: 420)
        .task { await load() }
    }

    /// Ask this Mac's own daemon what it is running.
    private func load() async {
        let result = await CLI.run(["--json", "status"])
        guard result.ok, let data = result.output.data(using: .utf8),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        daemon = DaemonBuild(
            version: body["daemonVersion"] as? String ?? "unknown",
            matches: body["buildsMatch"] as? Bool ?? true,
            platform: body["platform"] as? String ?? "")
    }
}
