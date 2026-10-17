import AgentKit
import SystemConfiguration
import SwiftUI

/// The runners this Mac can reach, and what the app knows about each.
///
/// A runner is one `farcoolerd`: one Unix user, on one host, with its own
/// worktrees. A host may carry several, which is why this is not called
/// "machines" — three engineers sharing a Linux box is three runners sharing
/// nothing.
///
/// This replaced a single free-text field. That field was honest about the
/// architecture — a runner is just an ssh target, and there is no Far Cooler
/// listener anywhere — but it made everything around it the user's problem:
/// whether the runner was reachable, whether Far Cooler was installed there,
/// whether what was installed matched this Mac, and what to type to fix any of
/// it. Every one of those questions has an answer the app can get.
///
/// Stored as a list rather than one string because the whole point of a fleet
/// is that it spans runners at once — there is no longer a single one being
/// "driven," so this class holds no selection, only the configured set.
@MainActor
final class Runners: ObservableObject {
    static let shared = Runners(storageKey: "hosts.configured")

    /// Every configured runner, in the order they were added.
    @Published private(set) var all: [Runner] = []

    /// The defaults key, unchanged by the rename: it names a slot on disk that
    /// existing installs already wrote, and renaming it would silently forget
    /// every runner anyone had configured.
    private let storageKey: String

    /// A list that reads and writes the defaults key `storageKey`. The shared
    /// one uses the key existing installs wrote; a test uses its own, so it
    /// never touches the person's real runners.
    init(storageKey: String) {
        self.storageKey = storageKey
        load()
    }

    /// Whether this Mac's own runner notifies, as far as it has been asked this
    /// run. Not stored: `push status` is the authority and answers in a moment.
    @Published private(set) var localPaired: Bool?

    /// For tests. A stand-in for the `farcooler` binary, so the arguments and
    /// stdin a pairing really sends can be read back from a file.
    var executableForTesting: String?
    /// For tests: the program that reads that stand-in, so a script written a
    /// moment ago is never executed as a program (`CLI.run`).
    var interpreterForTesting: String?
    /// For tests: a relay that is a script.
    var directoryForTesting: DirectoryClient?

    // MARK: - The list

    func add(_ target: String) {
        let trimmed = target.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty, !all.contains(where: { $0.target == trimmed }) else { return }
        all.append(Runner(target: trimmed))
        save()
    }

    func remove(_ target: String) {
        all.removeAll { $0.target == trimmed(target) }
        // The `~/.ssh/config` alias goes with it, or an editor opened on some
        // future runner with the same target string is handed a name Far Cooler
        // no longer maintains a block for. A direct runner's entry in the block is
        // left (removing it from this list is not removing this Mac's shell
        // access to it); a tunneled runner's is taken out below.
        SshConfigAliases.forget(trimmed(target))
        save()
        // A tunneled runner's entry in the block, and its token and host key,
        // go with it: removing a runner in Far Cooler removes what Far Cooler
        // wrote for it. A direct runner's entry is left, as it always was.
        let name = trimmed(target)
        Task { _ = await TunnelForget().forget(target: name) }
    }

    func record(_ probe: RunnerProbe, for target: String) {
        guard let index = all.firstIndex(where: { $0.target == target }) else { return }
        all[index].probe = probe
        all[index].lastError = nil
        save()
    }

    func record(error: String, for target: String) {
        guard let index = all.firstIndex(where: { $0.target == target }) else { return }
        all[index].lastError = error
        all[index].probe = nil
        save()
    }

    private func trimmed(_ target: String) -> String {
        target.trimmingCharacters(in: .whitespaces)
    }

    // MARK: - Asking runners about themselves

    /// This Mac's build stamp, for comparing against what a runner has installed.
    ///
    /// Read once: it cannot change while the app is running, and the answer
    /// costs a subprocess.
    static var localBuild: String = ""

    /// Ask a runner what it is. Changes nothing on it.
    func probe(_ target: String) async {
        if Self.localBuild.isEmpty {
            let version = await CLI.run(["--version"])
            Self.localBuild = version.output.trimmingCharacters(in: .whitespacesAndNewlines)
        }

        let result = await CLI.run(["--json", "host", "probe", target])
        guard result.ok, let data = result.output.data(using: .utf8),
            let decoded = try? JSONDecoder().decode(RunnerProbe.self, from: data)
        else {
            // The CLI's own words. It is the thing that talked to ssh, and its
            // message names what to fix — a refused connection, a missing key,
            // an unknown host — where anything written here would be a guess.
            record(error: result.said.isEmpty ? "Couldn’t reach this runner." : result.said,
                   for: target)
            return
        }
        record(decoded, for: target)
    }

    /// Install the daemon and CLI onto a runner, and register whatever keeps
    /// them running there.
    ///
    /// **This restarts the daemon on that runner**, which is not free: agent
    /// conversations live in the daemon's memory and do not survive it. See
    /// `DaemonSkew`. Every caller has to have said so first — Settings >
    /// Runners confirms before a Reinstall, and the sidebar's update card
    /// states the cost in full.
    ///
    /// Hands back whether it worked as well as what it said. The transcript
    /// alone cannot answer that: `runner install` prints its progress as it
    /// goes, so a run that got three steps in and then refused looks, in text,
    /// much like one that finished — and a caller that treated the two the
    /// same would report a runner updated when it is not.
    func install(_ target: String) async -> (ok: Bool, output: String) {
        let result = await CLI.run(["host", "install", target])
        return (result.ok, result.said)
    }

    // MARK: - Letting a runner reach you

    /// The teams a runner may join, personal first, for the picker.
    func teamsForJoining() async -> Result<[DirectoryTeam], DirectoryFailure> {
        await registration().teams()
    }

    /// Put a runner in a team and let it notify your devices.
    ///
    /// This is the front door to the whole notification product: an account
    /// with no paired runner posts no agent state, so the relay has nothing to
    /// push and no Live Activity can ever be raised. It used to ask the relay
    /// for a bare `/v1/daemons` token. It now registers the runner in the team
    /// directory (ov-528), which hands back the same kind of token, so
    /// notifications work as before and the directory also knows the runner.
    ///
    /// `moveKey` is the second ask: the relay said the runner is already yours
    /// in another team, the person confirmed, and this is the key it showed. It
    /// runs the runner's signed claim instead of its registration, and refuses
    /// a runner that signs with any other key.
    ///
    /// The sequence, and the rule that the token goes to the runner on stdin
    /// and nowhere else, are `RunnerRegistration`'s; this wires it to the
    /// bundled CLI. The token is never stored here, never logged, never shown.
    func join(_ target: String, team: DirectoryTeam, moveKey: String? = nil) async -> RunnerRegistration.Outcome {
        let flow = registration()
        let name = directoryName(for: target)
        let outcome = if let moveKey {
            await flow.move(target: target, team: team, directoryName: name, runnerKey: moveKey)
        } else {
            await flow.register(target: target, team: team, directoryName: name)
        }
        if outcome.succeeded { setPaired(true, for: target) }
        return outcome
    }

    /// What the team sees a runner called.
    ///
    /// The computer's own name for this Mac, and the runner's hostname from its
    /// probe for another one. The ssh target is a last resort and only its host
    /// part, because it can hold a username and an address that were never
    /// meant for the rest of a team.
    func directoryName(for target: String) -> String {
        Self.directoryName(
            target: target, hostname: all.first { $0.target == target }?.probe?.hostname,
            computerName: Self.computerName())
    }

    /// The name from System Settings > General ▸ Sharing. Read from the dynamic
    /// store, which answers from memory, rather than `Host.current()`, which
    /// resolves addresses and can stall the main actor for seconds.
    static func computerName() -> String? {
        SCDynamicStoreCopyComputerName(nil, nil) as String?
    }

    static func directoryName(target: String, hostname: String?, computerName: String?) -> String {
        func clean(_ value: String?) -> String? {
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return trimmed.isEmpty ? nil : trimmed
        }
        if target.isEmpty { return clean(computerName) ?? "Mac" }
        if let hostname = clean(hostname) { return hostname }
        let host = clean(target.split(separator: "@").last.map(String.init)) ?? ""
        // An address names nobody.
        // An IPv6 literal has a colon (and may be bracketed); an IPv4 one is
        // digits and dots. Neither is a name.
        let isAddress = host.contains(":") || host.allSatisfy { $0.isNumber || $0 == "." }
        return host.isEmpty || isAddress ? "Runner" : host
    }

    private func registration() -> RunnerRegistration {
        let executable = executableForTesting
        let interpreter = interpreterForTesting
        return RunnerRegistration(
            directory: directoryForTesting ?? Account.shared.directoryClient,
            relay: Account.shared.relay, defaultRelay: Account.defaultRelay,
            run: { arguments, stdin in
                let result = await CLI.run(arguments, stdin: stdin, executable: executable, interpreter: interpreter)
                return RunnerRegistration.CLIOutput(ok: result.ok, output: result.output, said: result.said)
            })
    }

    /// Whether this Mac's own runner is paired. Another runner's answer comes
    /// back in its probe, in the same ssh trip; this one is a local process.
    func refreshLocalPairing() async {
        let result = await CLI.run(["push", "status"], executable: executableForTesting, interpreter: interpreterForTesting)
        guard result.ok else { return }
        localPaired = Self.paired(inStatus: result.output)
    }

    /// `push status`'s answer, from its last line that says either thing, so a
    /// login shell's banner before it can't pass for "paired". Nil when it
    /// said neither.
    static func paired(inStatus output: String) -> Bool? {
        for line in output.split(whereSeparator: \.isNewline).reversed() {
            if line == "not paired" { return false }
            if line.hasPrefix("paired") { return true }
        }
        return nil
    }

    private func setPaired(_ paired: Bool, for target: String) {
        if target.isEmpty {
            localPaired = paired
            return
        }
        guard let index = all.firstIndex(where: { $0.target == target }) else { return }
        all[index].probe?.paired = paired
        save()
    }

    /// Stop a runner notifying you, from the runner's side.
    func unpairNotifications(_ target: String) async -> String {
        var arguments = target.isEmpty ? [] : ["--host", target]
        arguments += ["push", "forget"]
        let result = await CLI.run(arguments, executable: executableForTesting, interpreter: interpreterForTesting)
        if result.ok { setPaired(false, for: target) }
        return result.ok ? "This runner will no longer notify you." : result.said
    }

    // MARK: - Persistence
    //
    // `UserDefaults` through `Codable`, not `@AppStorage`: this is a list, and
    // `@AppStorage` only carries the property-list scalars.

    private func load() {
        guard let data = UserDefaults.standard.data(forKey: storageKey),
            let decoded = try? JSONDecoder().decode([Runner].self, from: data)
        else { return }
        all = decoded
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(all) else { return }
        UserDefaults.standard.set(data, forKey: storageKey)
    }
}

/// One configured runner, plus the last thing the app learned about it.
///
/// The probe is cached so the list can say something useful the moment it
/// opens. It is a memory of a past answer, not a live one — `RunnersSettings`
/// re-probes on appearance, and everything shown from here is dated by
/// construction.
///
/// This was `RemoteHost` rather than `Host`, because `Network.NWEndpoint.Host`
/// exists and is visible wherever that framework is imported, and the collision
/// did not fail loudly — it resolved to the system type and every member access
/// on it became an error about a type nobody wrote. `Runner` is the right word
/// anyway, and it has no such twin.
struct Runner: Codable, Identifiable, Equatable {
    var target: String
    var probe: RunnerProbe?
    var lastError: String?
    var id: String { target }
}

/// What `farcooler host probe --json` reports.
///
/// The CLI-side type is `runner_install::Probe`; this is the same shape read
/// back. The subcommand is still spelled `host` here on purpose — that is the
/// CLI's own surface, kept working for everything already in shell history and
/// in scripts, and the app has no reason to be the thing that breaks it.
///
/// Everything optional, because a runner that answers half the questions is
/// more useful than one this refuses to decode.
struct RunnerProbe: Codable, Equatable {
    var platform: String?
    var os: String?
    var arch: String?
    var kernel: String?
    var tmux: String?
    var persistence: String?
    var installedCli: String?
    var installedDaemon: String?
    var serviceActive: Bool?
    var lingering: Bool?
    var blockers: [String]?
    var installable: Bool?
    /// The host's own name, which is what the team sees the runner called.
    var hostname: String?
    /// Whether this channel's CLI there says it's paired, asked in the same
    /// ssh trip as the rest. Nil when it couldn't say.
    var paired: Bool?

    var isInstalled: Bool { installedCli != nil && installedDaemon != nil }

    /// Whether what is installed there was built from the same source as this.
    ///
    /// The reason the build stamp exists: two runners speaking the same
    /// protocol from different source behave like two different programs, and
    /// the symptom is a bug you already fixed still happening. Nil when there is
    /// nothing installed to compare.
    func matchesThisMac(_ local: String) -> Bool? {
        guard let installedCli else { return nil }
        return installedCli.contains(local) || local.contains(installedCli)
    }

    /// One line for the list, saying the most important true thing.
    var summary: String {
        if let blockers, !blockers.isEmpty { return blockers[0] }
        guard isInstalled else { return "Far Cooler isn’t installed here yet." }
        // The version first. A runner that is installed and running is the
        // normal case; which build it is running is the thing worth glancing
        // at, and the only place it was visible before was a warning that only
        // appeared once it was already wrong.
        // Just the stamp. `farcoolerd --version` prints "farcoolerd 0.1.0+sha"
        // and repeating the binary's name in a row that is already about that
        // runner is noise.
        let running = installedDaemon
            .map { $0.split(separator: " ").last.map(String.init) ?? $0 }
            .map { "\($0) · " } ?? ""
        switch persistence {
        case "systemd": return "\(running)kept running by systemd."
        case "launchd": return "\(running)kept running by launchd."
        case "onDemand":
            return "\(running)runs while you are connected — nothing here starts it on its own."
        default: return "\(running)installed."
        }
    }

    var platformLabel: String {
        switch platform {
        case "macos": return "macOS"
        case "linux": return "Linux"
        case "wsl": return "Linux on Windows (WSL)"
        default: return os ?? "Unknown"
        }
    }
}
