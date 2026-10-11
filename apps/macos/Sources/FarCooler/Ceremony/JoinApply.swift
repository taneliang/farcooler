import Foundation

/// What accepting a reply does on this Mac, apart from drawing it.
///
/// Lifted out of `JoinView` so the part that matters — which runners end up
/// usable, what each one is called, what was written where — runs in a test with
/// a fake CLI and a scratch `~/.ssh/config`, which a SwiftUI sheet cannot.
///
/// ## A tunneled runner is usable only when all three of these happened
///
/// 1. **Its token is kept** (`TunnelRunnerKeeper`), because the `ProxyCommand`
///    reads it from there and nowhere else.
/// 2. **Its `Host` block is in `~/.ssh/config`**, because that block is the only
///    way anything on this Mac — Far Cooler's own CLI, Zed, git — is told that
///    this runner is reached by a command.
/// 3. **It is added under that block's alias**, because the alias is the name
///    `ssh` resolves. The granting device's own name for the runner is not one
///    this Mac's config knows.
///
/// A runner that misses any of them is not added and is named, with what to do.
/// The alternative is a row that says "ready" for a runner the first connection
/// to which fails with ssh's own words, which was the failure a tunneled grant
/// had before this existed.
struct JoinApply {
    var keeper: TunnelRunnerKeeper
    var patternsInUse: @Sendable () -> Set<String> = { SshConfig.patternsInUse() }
    var writeConfig: @Sendable ([String], URL) throws -> Void = {
        try SshConfig.write($0, identity: $1)
    }
    var cli: String = SshConfig.bundledCLI

    struct Applied {
        var joined: Joined
        /// What to add to the runner list, in the reply's order: the alias for
        /// a tunneled runner and the granter's name for the rest.
        var targets: [String]
        /// Alias and target pairs the editor menu substitutes.
        var remembered: [(alias: String, target: String)]
        var configNote: String?
    }

    /// - Parameters:
    ///   - configIdentity: Key B's private key when this Mac is writing
    ///     `~/.ssh/config`, and nil when it is not.
    ///   - target: the name a direct runner is added under.
    func run(
        _ manifest: CeremonyManifest, configIdentity: URL?, target: (CeremonyRunner) -> String
    ) async -> Applied {
        let runners = manifest.runners.map(TunnelRunners.localized)
        var joined = Joined(runners)
        var usable = joined.reachable
        let tunneled = usable.filter(\.isTunneled)

        if !tunneled.isEmpty {
            if configIdentity == nil {
                joined = joined.losing(tunneled, because: .needsShellAccess)
            } else if !(await keeper.keep(tunneled)) {
                joined = joined.losing(tunneled, because: .couldNotKeep)
            }
            usable = joined.reachable
        }

        var aliases: [String: String] = [:]
        var remembered: [(alias: String, target: String)] = []
        var configNote: String?
        if let identity = configIdentity, !usable.isEmpty {
            // Nothing to write is not the same as writing nothing. The block is
            // written whole every time and an empty array removes it, so a reply
            // where every runner was left out would delete the block an earlier
            // one made.
            let resolution = SshConfig.aliases(for: usable, avoiding: patternsInUse())
            var lines: [String] = []
            var chosen: [(alias: String, runner: CeremonyRunner)] = []
            for runner in usable {
                guard let alias = resolution.aliases[runner.id] else { continue }
                lines += SshConfig.block(for: runner, alias: alias, identity: identity, cli: cli)
                chosen.append((alias, runner))
            }
            do {
                try writeConfig(lines, identity)
                // Remembered only after the file took the block: an alias for a
                // block that was never written sends an editor to a `Host` that
                // does not exist.
                aliases = resolution.aliases
                remembered = chosen.map { (alias: $0.alias, target: target($0.runner)) }
                configNote = resolution.message
            } catch {
                configNote = (error as? LocalizedError)?.errorDescription
                let lost = usable.filter(\.isTunneled)
                if !lost.isEmpty {
                    joined = joined.losing(lost, because: .couldNotWriteConfig)
                    usable = joined.reachable
                }
            }
        }

        let targets = usable.map { runner in
            runner.isTunneled ? (aliases[runner.id] ?? target(runner)) : target(runner)
        }
        return Applied(
            joined: joined, targets: targets, remembered: remembered, configNote: configNote)
    }
}

extension CeremonyRunner {
    var isTunneled: Bool {
        if case .tailcat = reach { return true }
        return false
    }
}

/// Why a tunneled runner was left out of what this Mac can use.
enum TunnelTrouble: Equatable {
    /// This Mac is not writing `~/.ssh/config`, and a tunneled runner is
    /// reached only through a `Host` block there.
    case needsShellAccess
    case couldNotKeep
    case couldNotWriteConfig

    func sentence(naming names: String) -> String {
        switch self {
        case .needsShellAccess:
            return "\(names) can only be reached through Far Cooler’s tunnel, which this Mac "
                + "uses through ssh. Add this Mac again with Zed, Git, and Terminal turned on."
        case .couldNotKeep:
            return "Far Cooler couldn’t save the tunnel to \(names) on this Mac. "
                + "Add this Mac again to try once more."
        case .couldNotWriteConfig:
            return "Far Cooler couldn’t finish setting up the tunnel to \(names), because "
                + "~/.ssh/config wasn’t updated. Add this Mac again once that’s fixed."
        }
    }
}
