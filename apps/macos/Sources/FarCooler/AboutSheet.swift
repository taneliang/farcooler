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
/// machines at once: a fleet-wide roundup here would either duplicate
/// Settings ▸ Machines (which already shows each machine's installed version
/// next to its name) or race it, and "what am I running" — the question this
/// window answers — is a question about the app in your hand, which runs on
/// exactly one machine regardless of how many others it is talking to.
struct AboutSheet: View {
    @Environment(\.dismiss) private var dismiss

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
                // default empty host: the default reads as "no machine to
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

            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(16)
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
