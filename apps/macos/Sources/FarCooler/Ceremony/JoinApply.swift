import Foundation

/// Where a joined runner is listed. A protocol so what the sheet adds, and when,
/// is tested without the app's real list.
@MainActor protocol RunnerList {
    func add(_ target: String)
}

extension Runners: RunnerList {}

/// Where the alias an editor opens a runner by is remembered.
@MainActor protocol AliasStore {
    func remember(_ alias: String, for target: String)
}

struct DefaultsAliasStore: AliasStore {
    func remember(_ alias: String, for target: String) {
        SshConfigAliases.remember(alias, for: target)
    }
}

/// What accepting a reply does on this Mac, apart from drawing it.
///
/// Lifted out of `JoinView` so the part that matters — which runners end up
/// usable, what each one is called, what was written where — runs in a test with
/// a fake CLI and a scratch `~/.ssh/config`, which a SwiftUI sheet cannot.
///
/// ## A tunneled runner is usable only when all of these happened
///
/// 1. **It is a runner `ssh` can be told about**: a user and a host key that are
///    shaped like one. A `User` with a space in it is a parse error for the whole
///    file, and this block is at the top of it.
/// 2. **Its `Host` entry is in `~/.ssh/config`**, because that entry is the only
///    way anything on this Mac — Far Cooler's own CLI, Zed, git — is told that
///    this runner is reached by a command. It is added to what is already in
///    Far Cooler's block, never in place of it.
/// 3. **Its token and pin are kept** (`TunnelRunnerKeeper`), because the
///    `ProxyCommand` and `KnownHostsCommand` read them from there and nowhere
///    else. Written after the entry, so a refused config stores nothing; and if
///    the keeper then fails, the entry goes back out.
/// 4. **It is added under that entry's alias**, because the alias is the name
///    `ssh` resolves.
///
/// A runner that misses any of them is not added and is named, with what to do.
struct JoinApply {
    var keeper: TunnelRunnerKeeper
    /// Where a tunneled runner's address is kept for the day Tailcat fails
    /// (ov-485). Written only once the runner is kept, so a runner left out
    /// leaves no address behind.
    var alternatives: RouteAlternatives = DefaultsRouteAlternatives()
    var patternsInUse: @Sendable () -> Set<String> = { SshConfig.patternsInUse() }
    var readEntries: @Sendable () -> [SshConfig.Entry] = { SshConfig.entries() }
    var writeConfig: @Sendable ([String], URL) throws -> Void = {
        try SshConfig.write($0, identity: $1)
    }
    /// The `ProxyCommand` word for the CLI, or nil when there is none to name.
    var program: @Sendable () -> String? = { StableCLI.refresh() ? StableCLI.word() : nil }

    struct Applied {
        var joined: Joined
        /// What to add to the runner list, in the reply's order: the alias for
        /// a tunneled runner and the granter's name for the rest.
        var targets: [String]
        /// Alias and target pairs the editor menu substitutes.
        var remembered: [(alias: String, target: String)]
        var configNote: String?
    }

    /// Run, then do what the sheet does with the answer: list the runners and
    /// remember their aliases. Here rather than in the view so a test can see
    /// what is added, and that nothing is when the write failed.
    @MainActor
    func settle(
        _ manifest: CeremonyManifest, configIdentity: URL?, target: @Sendable (CeremonyRunner) -> String,
        runners: some RunnerList, aliases: some AliasStore
    ) async -> Applied {
        let applied = await run(manifest, configIdentity: configIdentity, target: target)
        for target in applied.targets { runners.add(target) }
        for entry in applied.remembered { aliases.remember(entry.alias, for: entry.target) }
        return applied
    }

    /// - Parameters:
    ///   - configIdentity: Key B's private key when this Mac is writing
    ///     `~/.ssh/config`, and nil when it is not.
    ///   - target: the name a direct runner is added under.
    func run(
        _ manifest: CeremonyManifest, configIdentity: URL?, target: @Sendable (CeremonyRunner) -> String
    ) async -> Applied {
        var joined = Joined(manifest.runners.map(TunnelRunners.localized))
        func leave(_ runners: [CeremonyRunner], because trouble: TunnelTrouble) {
            guard !runners.isEmpty else { return }
            joined = joined.losing(runners, because: trouble)
        }

        leave(joined.reachable.filter { !Self.shapedLikeARunner($0) }, because: .unusableDetails)
        leave(
            joined.reachable.filter { $0.isTunneled && !Self.pinned($0) }, because: .noHostKey)

        var program: String?
        if joined.reachable.contains(where: \.isTunneled) {
            let tunneled = joined.reachable.filter(\.isTunneled)
            if configIdentity == nil {
                leave(tunneled, because: .needsShellAccess)
            } else if let word = self.program() {
                program = word
            } else {
                leave(tunneled, because: .noTool)
            }
        }

        var aliases: [String: String] = [:]
        var remembered: [(alias: String, target: String)] = []
        var configNote: String?
        if let identity = configIdentity, !joined.reachable.isEmpty {
            let before = readEntries()
            let usable = joined.reachable
            let newKeys = Set(usable.map(SshConfig.key(for:)))
            // Another runner's alias is taken, and so is anything ssh already
            // reads. A runner's own old entry is not: it is about to be replaced.
            let taken = patternsInUse().union(
                before.filter { !newKeys.contains($0.key) }.map(\.alias))
            let resolution = SshConfig.aliases(for: usable, avoiding: taken)
            let movedToAddress = Set(before.filter(\.onItsAddress).map(\.key))
            let fresh = usable.compactMap { runner -> SshConfig.Entry? in
                guard let alias = resolution.aliases[runner.id] else { return nil }
                var entry = SshConfig.entry(
                    for: runner, alias: alias, identity: identity, program: program ?? "")
                // Granted again, it keeps the route a person chose (ov-485
                // L3): Connect Directly sticks until Use Tailcat.
                if movedToAddress.contains(entry.key), let address = Self.usableAlternative(runner) {
                    entry.lines = RouteSwitch.onItsAddress(entry.lines, host: address.host, port: address.port)
                    entry.onItsAddress = true
                }
                return entry
            }
            let kept = before.filter { !newKeys.contains($0.key) }
            do {
                try writeConfig(SshConfig.render(kept + fresh), identity)
                aliases = resolution.aliases
                configNote = resolution.message
                let tunneled = usable.filter(\.isTunneled)
                if !tunneled.isEmpty, !(await keeper.keep(tunneled)) {
                    // Back out to what was there, plus this reply's direct entries:
                    // a runner granted again keeps the entry that works.
                    let direct = fresh.filter { !$0.tunneled }
                    let directKeys = Set(direct.map(\.key))
                    let backedOut = before.filter { !directKeys.contains($0.key) } + direct
                    do {
                        try writeConfig(SshConfig.render(backedOut), identity)
                    } catch {
                        configNote = (error as? LocalizedError)?.errorDescription
                    }
                    leave(tunneled, because: .couldNotKeep)
                } else {
                    for runner in tunneled {
                        // Only an address `ssh_config` can hold (M1): it is
                        // written as `HostName`/`Port` on Connect Directly, in
                        // the first block of the file every `ssh` reads.
                        if let address = Self.usableAlternative(runner) {
                            alternatives.remember(host: address.host, port: address.port, for: runner.id)
                        } else {
                            alternatives.forget(runner.id)
                        }
                    }
                }
                // Remembered only after the file took the block: an alias for a
                // block that was never written sends an editor to a `Host` that
                // does not exist.
                let lost = Set(joined.lost.map(\.runner.id))
                remembered = usable.filter { !lost.contains($0.id) }.compactMap { runner in
                    resolution.aliases[runner.id].map { (alias: $0, target: target(runner)) }
                }
            } catch {
                let said = (error as? LocalizedError)?.errorDescription
                let tunneled = usable.filter(\.isTunneled)
                // A tunneled runner that was not added needs no promise that Zed
                // and git will find it later.
                configNote = tunneled.isEmpty ? said : said.map(Self.withoutThePromise)
                leave(tunneled, because: .couldNotWriteConfig)
            }
        }

        let targets = joined.reachable.map { runner in
            runner.isTunneled ? (aliases[runner.id] ?? target(runner)) : target(runner)
        }
        return Applied(
            joined: joined, targets: targets, remembered: remembered, configNote: configNote)
    }

    /// The cause, without "The keys were enrolled; Zed and Git won’t find the
    /// runners until it can." which is about direct runners that are listed.
    static func withoutThePromise(_ said: String) -> String {
        said.components(separatedBy: " The keys were enrolled").first ?? said
    }

    /// A user and an address `ssh` will read as one word each. Anything else is
    /// a line the whole file fails to parse on.
    static func shapedLikeARunner(_ runner: CeremonyRunner) -> Bool {
        guard runner.user.range(of: "^[A-Za-z_][A-Za-z0-9_.-]{0,31}$", options: .regularExpression)
            != nil
        else { return false }
        guard case .direct(let host, let port) = runner.reach else { return true }
        return host.range(of: "^[A-Za-z0-9][A-Za-z0-9._:-]{0,252}$", options: .regularExpression)
            != nil && (1...65535).contains(port)
    }

    /// A tunneled runner's alternative, when it is one host word and a port
    /// from 1 to 65535; nil for anything else, which is then never kept.
    static func usableAlternative(_ runner: CeremonyRunner) -> (host: String, port: Int)? {
        guard case .direct(let host, let port) = runner.alternative, (1...65535).contains(port),
            host.range(of: "^[A-Za-z0-9._:][A-Za-z0-9._:-]{0,252}$", options: .regularExpression) != nil
        else { return nil }
        return (host, port)
    }

    /// `SHA256:` and 43 base64 characters, which is what `ssh-keygen -l` prints
    /// and what `runner known-host` compares.
    static func pinned(_ runner: CeremonyRunner) -> Bool {
        runner.host_key.range(
            of: "^SHA256:[A-Za-z0-9+/]{43}$", options: .regularExpression) != nil
    }
}

extension CeremonyRunner {
    var isTunneled: Bool {
        if case .tailcat = reach { return true }
        return false
    }
}

/// Why a runner was left out of what this Mac can use.
enum TunnelTrouble: Equatable {
    /// This Mac is not writing `~/.ssh/config`, and a tunneled runner is
    /// reached only through a `Host` entry there.
    case needsShellAccess
    case couldNotKeep
    case couldNotWriteConfig
    /// A user or address `ssh` would choke on.
    case unusableDetails
    /// The granting device sent no host key to pin.
    case noHostKey
    /// There is no CLI to name in the `ProxyCommand`.
    case noTool

    func sentence(naming names: String, plural: Bool = false) -> String {
        let was = plural ? "weren’t" : "wasn’t"
        switch self {
        case .needsShellAccess:
            return "To use \(names), Far Cooler needs to add \(plural ? "them" : "it") to ~/.ssh/config. Add this Mac "
                + "again with “Allow Zed, Git, and Terminal to connect to these runners” and "
                + "“Add to ~/.ssh/config” turned on."
        case .couldNotKeep:
            return "Far Cooler couldn’t set up the tunnel to \(names) on this Mac. "
                + "Add this Mac again to try again."
        case .couldNotWriteConfig:
            return "\(names) \(was) added, because Far Cooler couldn’t update ~/.ssh/config."
        case .unusableDetails:
            return "\(names) \(was) added, because its details aren’t ones ssh can use."
        case .noHostKey:
            return "\(names) \(was) added, because the other device didn’t share its host key. "
                + "Add this Mac again from a device that can reach it."
        case .noTool:
            return "\(names) \(was) added, because Far Cooler couldn’t find its command-line tool. "
                + "Reinstall Far Cooler, then add this Mac again."
        }
    }
}
