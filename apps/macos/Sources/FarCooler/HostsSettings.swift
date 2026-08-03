import AgentKit
import SwiftUI

/// Add a machine, see what it is, install onto it, drive it.
///
/// The old settings pane was one text field and a paragraph explaining that
/// Far Cooler reaches other machines over ssh. That explanation is still true and
/// still worth making — there is no listener to set up anywhere — but it left
/// every consequence to the user: whether the machine was reachable, whether
/// Far Cooler was installed, whether the install matched this Mac, and what to
/// type to fix any of it.
///
/// So this asks the host instead. `farcooler host probe` changes nothing and
/// answers all four, which is what makes it safe to run on appearance and what
/// makes the install button able to say what it is about to do.
struct HostsSettings: View {
    @ObservedObject private var hosts = Hosts.shared

    @State private var newTarget = ""
    @State private var busy: Set<String> = []
    @State private var log: [String: String] = [:]

    var body: some View {
        Form {
            Section {
                localRow
                ForEach(hosts.all) { host in
                    hostRow(host)
                }
            } header: {
                Text("Machines")
            } footer: {
                Text(
                    "A user@host or an ssh config alias. Far Cooler reaches every machine "
                        + "over ssh — there is no Far Cooler listener to set up anywhere, so a "
                        + "host you can ssh to is a host you can drive."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }

            Section {
                HStack {
                    TextField("user@host, or an ssh alias", text: $newTarget)
                        .autocorrectionDisabled()
                        .onSubmit(addHost)
                    Button("Add", action: addHost)
                        .disabled(newTarget.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
        .formStyle(.grouped)
        .task {
            // Everything at once. These are independent ssh round trips and a
            // fleet of five machines should not take five times as long to
            // describe as one.
            await withTaskGroup(of: Void.self) { group in
                for host in hosts.all {
                    group.addTask { await probe(host.target) }
                }
            }
        }
    }

    // MARK: - Rows

    /// This Mac, in the same list as everything else.
    ///
    /// Not a special case at the top of the window: "no remote host" IS a
    /// choice of machine, and hiding that behind an empty text field is what
    /// made the old setting hard to read.
    private var localRow: some View {
        // A VStack, so a message about this machine is a LINE in the row rather
        // than an overlay hanging off the bottom of it. It was the latter, and
        // an overlay offset past its own bounds is one the form clips — so the
        // one thing this row had to say ("Sign in first") arrived half cut off.
        // `hostRow` had it right; this did not match it.
        VStack(alignment: .leading, spacing: 6) {
            localHeader
            if let message = log[""] {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, 2)
    }

    private var localHeader: some View {
        HStack {
            Image(systemName: "laptopcomputer")
                .foregroundStyle(.secondary)
                .frame(width: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text("This Mac").font(.body)
                Text("Local — nothing to install.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            // This Mac gets the same pairing control as anything else. Its
            // daemon is the same daemon and reaches a sleeping phone by the same
            // route — being local buys it no special path.
            Menu {
                Button("Notify me from this machine") { Task { await pair("") } }
                Button("Stop notifications from this machine") { Task { await unpair("") } }
            } label: {
                Image(systemName: "bell")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()

            if hosts.active.isEmpty {
                Label("Driving", systemImage: "checkmark.circle.fill")
                    .labelStyle(.iconOnly)
                    .foregroundStyle(.green)
            } else {
                Button("Drive") { hosts.active = "" }
            }
        }
    }

    private func hostRow(_ host: RemoteHost) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Image(systemName: icon(for: host))
                    .foregroundStyle(.secondary)
                    .frame(width: 20)

                VStack(alignment: .leading, spacing: 2) {
                    Text(host.target).font(.body)
                    Text(subtitle(for: host))
                        .font(.caption)
                        .foregroundStyle(
                            host.lastError == nil
                                ? AnyShapeStyle(.secondary) : AnyShapeStyle(Color.orange))
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }

                Spacer()

                if busy.contains(host.target) {
                    ProgressView().controlSize(.small)
                } else {
                    actions(for: host)
                }
            }

            if let output = log[host.target] {
                // The installer's own words, not a paraphrase. It is the thing
                // that knows what went wrong.
                DetailBox(text: output)
            }

            if let probe = host.probe, probe.isInstalled,
                let matches = probe.matchesThisMac(Hosts.localBuild), matches == false
            {
                Label(
                    "Built from different source than this Mac. Install again to match.",
                    systemImage: "exclamationmark.triangle.fill"
                )
                .font(.caption)
                .foregroundStyle(.orange)
            }
        }
        .padding(.vertical, 2)
    }

    @ViewBuilder
    private func actions(for host: RemoteHost) -> some View {
        HStack(spacing: 8) {
            if hosts.active == host.target {
                Label("Driving", systemImage: "checkmark.circle.fill")
                    .labelStyle(.iconOnly)
                    .foregroundStyle(.green)
            } else if host.probe?.isInstalled == true {
                Button("Drive") { hosts.active = host.target }
            }

            if host.probe?.installable == true {
                Button(host.probe?.isInstalled == true ? "Reinstall" : "Install") {
                    Task { await install(host.target) }
                }
            }

            Menu {
                Button("Check again") { Task { await probe(host.target) } }
                Section {
                    Button("Notify me from this machine") {
                        Task { await pair(host.target) }
                    }
                    Button("Stop notifications from this machine") {
                        Task { await unpair(host.target) }
                    }
                }
                Button("Remove", role: .destructive) { hosts.remove(host.target) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
    }

    private func icon(for host: RemoteHost) -> String {
        switch host.probe?.platform {
        case "macos": return "desktopcomputer"
        case "wsl": return "pc"
        case "linux": return "server.rack"
        default: return host.lastError == nil ? "network" : "exclamationmark.triangle"
        }
    }

    private func subtitle(for host: RemoteHost) -> String {
        if let error = host.lastError { return error }
        guard let probe = host.probe else { return "Checking…" }
        return "\(probe.platformLabel) · \(probe.summary)"
    }

    // MARK: - Doing things to machines

    private func addHost() {
        let target = newTarget.trimmingCharacters(in: .whitespaces)
        guard !target.isEmpty else { return }
        hosts.add(target)
        newTarget = ""
        Task { await probe(target) }
    }

    private func probe(_ target: String) async {
        busy.insert(target)
        defer { busy.remove(target) }
        await hosts.probe(target)
    }

    private func pair(_ target: String) async {
        busy.insert(target)
        defer { busy.remove(target) }
        log[target] = await hosts.pairForNotifications(target)
    }

    private func unpair(_ target: String) async {
        busy.insert(target)
        defer { busy.remove(target) }
        log[target] = await hosts.unpairNotifications(target)
    }

    private func install(_ target: String) async {
        busy.insert(target)
        log[target] = nil
        defer { busy.remove(target) }

        // Shown whether it worked or not. An install that failed halfway is
        // exactly when the transcript matters, and it is the only place the
        // reason exists.
        log[target] = await hosts.install(target)
        await hosts.probe(target)
    }
}
