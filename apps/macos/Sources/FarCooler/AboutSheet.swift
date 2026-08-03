import AgentKit
import SwiftUI

/// About Far Cooler — which build this is, and which one it is driving.
///
/// Replaces the standard panel rather than decorating it. That panel shows
/// `CFBundleShortVersionString` and `CFBundleVersion`, which for a beta and the
/// release it names are identical — so the window whose entire job is answering
/// "what am I running" could not. This one names the channel, and the daemon on
/// the machine currently being driven, because those two are what have to match.
struct AboutSheet: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject private var hosts = Hosts.shared

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
                VersionSection(daemon: daemon, host: hosts.active) { text in
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

    /// Ask the machine being driven what it is running.
    ///
    /// Through the CLI's `status --json`, which is the same call the Machines
    /// screen uses, so the two cannot come to disagree about the same host.
    private func load() async {
        var arguments = hosts.active.isEmpty ? [] : ["--host", hosts.active]
        arguments += ["--json", "status"]
        let result = await CLI.run(arguments)
        guard result.ok, let data = result.output.data(using: .utf8),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        daemon = DaemonBuild(
            version: body["daemonVersion"] as? String ?? "unknown",
            matches: body["buildsMatch"] as? Bool ?? true,
            platform: body["platform"] as? String ?? "")
    }
}
