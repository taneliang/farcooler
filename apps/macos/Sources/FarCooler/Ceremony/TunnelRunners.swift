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
    /// the runner: `you@box` on a Mac, anything at all from a scanned code, and
    /// empty for the granter's own Mac. It is typed into a line `ssh` hands to a
    /// shell, so it is not an id this Mac may take on trust, and two machines
    /// that call themselves `box` would share one.
    ///
    /// Hashed from what the runner owns instead: the host key its sshd presents
    /// and the Unix user it is. Those are the two facts that make a runner one
    /// runner (see `docs/runners.md`: a host may carry several, one per user), the
    /// granter pins them, and granting the same runner again from anywhere gives
    /// the same id, so it replaces its entry rather than adding another. A box
    /// that was reinstalled has a new host key and so a new id.
    static func localID(for runner: CeremonyRunner) -> String {
        let digest = SHA256.hash(data: Data((runner.host_key + "\n" + runner.user).utf8))
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
            return ["id": $0.id, "token": token, "host_key": $0.host_key]
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
    /// Drop these ids, token and pin with them. True when none is left.
    func forget(_ ids: [String]) async -> Bool
}

extension CLINodeKeyTool: TunnelRunnerKeeper {
    func keep(_ runners: [CeremonyRunner]) async -> Bool {
        // Nothing to send is only a success when nothing was asked for.
        guard let payload = TunnelRunners.payload(for: runners) else { return runners.isEmpty }
        return await attempt(["runner", "store-runners"], stdin: payload, doing: "store runners")
            != nil
    }

    func forget(_ ids: [String]) async -> Bool {
        guard !ids.isEmpty else { return true }
        guard let payload = try? JSONSerialization.data(withJSONObject: ids) else { return false }
        return await attempt(["runner", "forget-runners"], stdin: payload, doing: "forget runners")
            != nil
    }
}
