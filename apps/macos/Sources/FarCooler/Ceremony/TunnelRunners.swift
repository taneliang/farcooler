import CryptoKit
import Foundation

/// The runners this Mac was granted through the tunnel, and where they are
/// kept so a `ProxyCommand` can find them.
///
/// A tunneled runner has no address, so what `ssh` is given for it is a command
/// (`farcooler runner pipe <id>`, see ``SshConfig``), and that command runs with
/// no app around it. It reads the token from `runners.json` in the runtime
/// directory — a file only `farcooler runner store-runners` writes, so the same
/// CLI that reads it is the only thing that decides its shape. The token never
/// goes into `~/.ssh/config`, which every editor on this Mac reads.
enum TunnelRunners {
    /// The id a `ProxyCommand` names for a runner, made here.
    ///
    /// **Never the granting device's id.** That is whatever the granter calls
    /// the runner: `you@box` on a Mac, anything at all from a scanned code. It
    /// is typed into a line `ssh` hands to a shell, so it is not an id this
    /// Mac may take on trust; and an empty one (the granter's own Mac) names
    /// nothing. A hash is a fixed, shell-inert spelling that is still the same
    /// every time the same runner is granted, so granting it again replaces
    /// the entry rather than adding another.
    static func localID(for runner: CeremonyRunner) -> String {
        let name = runner.id.isEmpty ? "label:" + runner.label : runner.id
        let digest = SHA256.hash(data: Data(name.utf8))
        return "fc-" + digest.prefix(8).map { String(format: "%02x", $0) }.joined()
    }

    /// `runner` with the id this Mac names it by, if it is a tunneled one. A
    /// direct runner keeps the id it arrived with: nothing about it is typed
    /// into a shell.
    static func localized(_ runner: CeremonyRunner) -> CeremonyRunner {
        guard case .tailcat = runner.reach else { return runner }
        var local = runner
        local.id = localID(for: runner)
        return local
    }

    /// The entries `store-runners` reads on stdin: `[{"id","token"}]`.
    static func payload(for runners: [CeremonyRunner]) -> Data? {
        let entries: [[String: String]] = runners.compactMap {
            guard case .tailcat(let token) = $0.reach else { return nil }
            return ["id": $0.id, "token": token]
        }
        guard !entries.isEmpty else { return nil }
        return try? JSONSerialization.data(withJSONObject: entries)
    }
}

/// Where the granted runners are kept. A protocol so what the join screen does
/// when this fails is tested without a CLI.
protocol TunnelRunnerKeeper: Sendable {
    /// True only when every runner in `runners` (already localized) is stored.
    func keep(_ runners: [CeremonyRunner]) async -> Bool
}

extension CLINodeKeyTool: TunnelRunnerKeeper {
    func keep(_ runners: [CeremonyRunner]) async -> Bool {
        guard let payload = TunnelRunners.payload(for: runners) else { return true }
        return await attempt(["runner", "store-runners"], stdin: payload, doing: "store runners")
            != nil
    }
}
