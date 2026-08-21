import AgentKit
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
    static let shared = Runners()

    /// Every configured runner, in the order they were added.
    @Published private(set) var all: [Runner] = []

    /// The defaults key, unchanged by the rename: it names a slot on disk that
    /// existing installs already wrote, and renaming it would silently forget
    /// every runner anyone had configured.
    private let storageKey = "hosts.configured"

    private init() {
        load()
    }

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
        // no longer maintains a block for. The block itself is a separate
        // operation with its own copy — removing a runner from this list is not
        // removing this Mac's shell access to it.
        SshConfigAliases.forget(trimmed(target))
        save()
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
            record(error: result.output.isEmpty ? "Couldn't reach this runner." : result.output,
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
    /// `DaemonSkew`. Every caller has to have said so first — Settings ▸
    /// Runners confirms before a Reinstall, and the sidebar's update card
    /// states the cost in full.
    ///
    /// Hands back whether it worked as well as what it said. The transcript
    /// alone cannot answer that: `runner install` prints its progress as it
    /// goes, so a run that got three steps in and then refused looks, in text,
    /// much like one that finished — and a caller that treated the two the
    /// same would report a runner updated when it is not.
    func install(_ target: String) async -> (ok: Bool, output: String) {
        await CLI.run(["host", "install", target])
    }

    // MARK: - Letting a runner reach you

    /// Give a runner a token so it can notify your devices, and nothing else.
    ///
    /// Two steps that must not be split: ask the relay for a token, then hand it
    /// to the runner over ssh. The token exists in readable form for exactly
    /// the span between those two lines — the relay keeps only a hash — so it is
    /// never stored here, never logged, and never shown.
    ///
    /// The runner itself does no signing in. That is the whole point: a
    /// headless Linux box has no browser, and a token that names an account and
    /// carries no destination is worth only "notify its own owner".
    func pairForNotifications(_ target: String) async -> String {
        guard Account.shared.isSignedIn else {
            return "Sign in first — Settings ▸ Account."
        }
        let label = target.isEmpty ? "This Mac" : target
        guard let token = await Account.shared.pairDaemon(label: label) else {
            return "The relay wouldn't issue a token. Try signing in again."
        }

        var arguments = target.isEmpty ? [] : ["--host", target]
        arguments += ["push", "pair"]
        // Down a pipe, never as an argument. `ps` is readable by every process
        // running as this user, and the CLI forwards the same way over ssh, so
        // a token passed here would be visible on both ends for the life of
        // the command.
        let result = await CLI.run(arguments, stdin: token)
        // The CLI's own words on failure, and a fixed sentence on success — the
        // success output would otherwise be the only place the token could
        // surface, and it must not.
        return result.ok ? "This runner can now notify your devices." : result.output
    }

    /// Stop a runner notifying you, from the runner's side.
    func unpairNotifications(_ target: String) async -> String {
        var arguments = target.isEmpty ? [] : ["--host", target]
        arguments += ["push", "forget"]
        let result = await CLI.run(arguments)
        return result.ok ? "This runner will no longer notify you." : result.output
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
        guard isInstalled else { return "Far Cooler is not installed here yet." }
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
