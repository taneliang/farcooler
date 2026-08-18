import AgentKit
import SwiftUI

/// Add a runner, see what it is, install onto it.
///
/// The old settings pane was one text field and a paragraph explaining that
/// Far Cooler reaches other runners over ssh. That explanation is still true and
/// still worth making — there is no listener to set up anywhere — but it left
/// every consequence to the user: whether the runner was reachable, whether
/// Far Cooler was installed, whether the install matched this Mac, and what to
/// type to fix any of it.
///
/// So this asks the runner instead. `farcooler host probe` changes nothing and
/// answers all four, which is what makes it safe to run on appearance and what
/// makes the install button able to say what it is about to do.
struct RunnersSettings: View {
    @ObservedObject private var runners = Runners.shared

    @State private var newTarget = ""
    @State private var busy: Set<String> = []
    @State private var log: [String: String] = [:]
    /// The runner whose settings sheet is open, by ssh target — `""` for this
    /// Mac, which is a real value here and not "nothing".
    ///
    /// Optional-of-target rather than a bool plus a target, so the sheet cannot
    /// be presented without knowing which runner it is about.
    @State private var editingRunner: RunnerChoice?

    var body: some View {
        Form {
            Section {
                localRow
                ForEach(runners.all) { runner in
                    runnerRow(runner)
                }
            } header: {
                HStack {
                    Text("Runners")
                    Spacer()
                    // Per-runner dots already retry one runner with a click;
                    // this is the same `reconnectNow()` for every one of
                    // them at once — the escape hatch for "I fixed the VPN"
                    // without waiting out however many backoffs are
                    // currently ticking down, or clicking each dot in turn.
                    Button("Reconnect all") { Reachability.shared.retryNow() }
                        .font(.caption)
                        .buttonStyle(.link)
                }
            } footer: {
                Text("Connect to any runner you can reach over SSH.")
            }

            Section {
                HStack {
                    TextField("user@host, or an ssh alias", text: $newTarget)
                        .autocorrectionDisabled()
                        .onSubmit(addRunner)
                    Button("Add", action: addRunner)
                        .disabled(newTarget.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
        .formStyle(.grouped)
        .sheet(item: $editingRunner) { choice in
            RunnerSettingsSheet(name: choice.name, target: choice.target)
        }
        .task {
            // Everything at once. These are independent ssh round trips and a
            // fleet of five runners should not take five times as long to
            // describe as one.
            await withTaskGroup(of: Void.self) { group in
                for runner in runners.all {
                    group.addTask { await probe(runner.target) }
                }
            }
        }
    }

    // MARK: - Runner settings

    /// Opens one runner's own `config.toml` as a screen.
    ///
    /// A sheet rather than a disclosure in this row: the theme editor inside it
    /// is nineteen color wells over a live terminal preview, and this window is
    /// 520 points wide.
    private func settingsButton(name: String, target: String) -> some View {
        Button {
            editingRunner = RunnerChoice(name: name, target: target)
        } label: {
            Image(systemName: "slider.horizontal.3")
        }
        .buttonStyle(.borderless)
        .help("Settings on \(name)")
    }

    // MARK: - Rows

    /// This Mac, in the same list as everything else.
    ///
    /// Not a special case at the top of the window: "no remote runner" IS a
    /// choice of runner, and hiding that behind an empty text field is what
    /// made the old setting hard to read.
    private var localRow: some View {
        // A VStack, so a message about this runner is a LINE in the row rather
        // than an overlay hanging off the bottom of it. It was the latter, and
        // an overlay offset past its own bounds is one the form clips — so the
        // one thing this row had to say ("Sign in first") arrived half cut off.
        // `runnerRow` had it right; this did not match it.
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
            settingsButton(name: "This Mac", target: "")
            Menu {
                Button("Notify me from this runner") { Task { await pair("") } }
                Button("Stop notifications from this runner") { Task { await unpair("") } }
            } label: {
                Image(systemName: "bell")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
    }

    private func runnerRow(_ runner: Runner) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Image(systemName: icon(for: runner))
                    .foregroundStyle(.secondary)
                    .frame(width: 20)

                VStack(alignment: .leading, spacing: 2) {
                    Text(runner.target).font(.body)
                    Text(subtitle(for: runner))
                        .font(.caption)
                        .foregroundStyle(
                            runner.lastError == nil
                                ? AnyShapeStyle(.secondary) : AnyShapeStyle(Color.orange))
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }

                Spacer()

                if busy.contains(runner.target) {
                    ProgressView().controlSize(.small)
                } else {
                    settingsButton(name: runner.target, target: runner.target)
                    actions(for: runner)
                }
            }

            if let output = log[runner.target] {
                // The installer's own words, not a paraphrase. It is the thing
                // that knows what went wrong.
                DetailBox(text: output)
            }

            if let probe = runner.probe, probe.isInstalled,
                let matches = probe.matchesThisMac(Runners.localBuild), matches == false
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
    private func actions(for runner: Runner) -> some View {
        HStack(spacing: 8) {
            if runner.probe?.installable == true {
                Button(runner.probe?.isInstalled == true ? "Reinstall" : "Install") {
                    Task { await install(runner.target) }
                }
            }

            Menu {
                Button("Check again") { Task { await probe(runner.target) } }
                Section {
                    Button("Notify me from this runner") {
                        Task { await pair(runner.target) }
                    }
                    Button("Stop notifications from this runner") {
                        Task { await unpair(runner.target) }
                    }
                }
                Button("Remove", role: .destructive) { runners.remove(runner.target) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
    }

    private func icon(for runner: Runner) -> String {
        switch runner.probe?.platform {
        case "macos": return "desktopcomputer"
        case "wsl": return "pc"
        case "linux": return "server.rack"
        default: return runner.lastError == nil ? "network" : "exclamationmark.triangle"
        }
    }

    private func subtitle(for runner: Runner) -> String {
        if let error = runner.lastError { return error }
        guard let probe = runner.probe else { return "Checking…" }
        return "\(probe.platformLabel) · \(probe.summary)"
    }

    // MARK: - Doing things to runners

    private func addRunner() {
        let target = newTarget.trimmingCharacters(in: .whitespaces)
        guard !target.isEmpty else { return }
        runners.add(target)
        newTarget = ""
        Task { await probe(target) }
    }

    private func probe(_ target: String) async {
        busy.insert(target)
        defer { busy.remove(target) }
        await runners.probe(target)
    }

    private func pair(_ target: String) async {
        busy.insert(target)
        defer { busy.remove(target) }
        log[target] = await runners.pairForNotifications(target)
    }

    private func unpair(_ target: String) async {
        busy.insert(target)
        defer { busy.remove(target) }
        log[target] = await runners.unpairNotifications(target)
    }

    private func install(_ target: String) async {
        busy.insert(target)
        log[target] = nil
        defer { busy.remove(target) }

        // Shown whether it worked or not. An install that failed halfway is
        // exactly when the transcript matters, and it is the only place the
        // reason exists.
        log[target] = await runners.install(target)
        await runners.probe(target)

        // `runners.probe` only updates this row's own display. The
        // `DaemonClient` everything else reads through — the sidebar, "Add
        // repository", every `store.refusal(for:)` — is a separate object
        // that landed in `.notInstalled` before this install ran, and by
        // design does not recheck for minutes at a time so a runner that
        // genuinely has no Far Cooler is not hammered forever. Without this,
        // an install that just succeeded here still refuses everywhere else
        // until that backoff happens to elapse. The same call "Reconnect
        // all" makes.
        Reachability.shared.retryNow()
    }
}

/// Which runner a settings sheet is about.
///
/// A type rather than a bare `String?`, because `""` is a real target — this Mac
/// — and an optional string could not tell it apart from "no runner chosen".
struct RunnerChoice: Identifiable, Hashable {
    let name: String
    let target: String
    var id: String { target }
}
