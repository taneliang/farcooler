import Foundation

/// Where a remote runner is, for the client core to dial it (ov-408): what
/// `ssh` itself would use for the target, and the host keys the owner's
/// `known_hosts` already trusts for it.
///
/// **Read, never written.** `ssh -G` reads the owner's config (every `Host`,
/// `Match` and `Include`); it opens no connection, though a config may still
/// resolve names (`CanonicalizeHostname`) or run its own `Match exec`.
/// `ssh-keygen -F` reads `known_hosts`. Nothing here writes a file in `~/.ssh`, and nothing does a
/// keyscan: trusting whatever answers now is the unknown-host prompt with the
/// human taken out. A runner the owner has never reached with `ssh` has no key
/// here, and the conversation view isn't offered on it.
///
/// The same tools `RunnerFacts` asks for a ceremony's reply, with the port, a
/// `HostKeyAlias` and the config's own `known_hosts` files taken into account,
/// since this Mac is the one dialing.
struct RemoteReach: Equatable, Sendable {
    let host: String
    let port: Int
    let user: String
    /// Every `SHA256:…` the owner's `known_hosts` trusts for this runner.
    let knownKeys: Set<String>
    /// The ones marked `@revoked`, which nothing here ever pins.
    var revokedKeys: Set<String> = []

    /// Why a runner can't be dialed by the client core. Each is a sentence for
    /// Settings, never ssh's own words.
    enum Refusal: Error, Equatable, Sendable {
        /// `ssh -G` named no host: the target isn't one ssh can resolve.
        case unresolved
        /// Reached through `ProxyJump` or `ProxyCommand`, which the core
        /// can't follow.
        case proxied
        /// No key for it in `known_hosts`.
        case unknownHost

        var sentence: String {
            switch self {
            case .unresolved: "Far Cooler couldn’t look up this runner’s address in your ssh settings."
            case .proxied: "This runner is reached through a proxy, so its Claude panes show the terminal."
            case .unknownHost: "Connect to this runner once with ssh, so Far Cooler can check its host key."
            }
        }
    }

    /// `ssh -G`'s answer, as `keyword value` lines, keywords lowercased.
    static func settings(_ output: String) -> [String: String] {
        var settings: [String: String] = [:]
        for line in output.components(separatedBy: .newlines) {
            let parts = line.split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
            guard parts.count == 2 else { continue }
            settings[parts[0].lowercased()] = String(parts[1])
        }
        return settings
    }

    /// Where the core would dial, from `ssh -G`'s settings, or why it can't.
    static func destination(_ settings: [String: String]) -> Result<(host: String, port: Int, user: String), Refusal> {
        for keyword in ["proxyjump", "proxycommand"] {
            if let value = settings[keyword], !value.isEmpty, value.lowercased() != "none" {
                return .failure(.proxied)
            }
        }
        guard let host = settings["hostname"], !host.isEmpty else { return .failure(.unresolved) }
        let port = Int(settings["port"] ?? "") ?? 22
        let user = settings["user"].flatMap { $0.isEmpty ? nil : $0 } ?? NSUserName()
        return .success((host, port, user))
    }

    /// The name `known_hosts` files this runner under: the `HostKeyAlias` if
    /// the config sets one, else the host, bracketed with its port when the
    /// port isn't 22. ssh's own rule, so the lookup finds what ssh would.
    static func knownHostsName(_ settings: [String: String], host: String, port: Int) -> String {
        if let alias = settings["hostkeyalias"], !alias.isEmpty, alias.lowercased() != "none" { return alias }
        return port == 22 ? host : "[\(host)]:\(port)"
    }

    /// The `known_hosts` files ssh reads for this runner, the user's first.
    static func knownHostsFiles(_ settings: [String: String]) -> [String] {
        ["userknownhostsfile", "globalknownhostsfile"]
            .flatMap { (settings[$0] ?? "").split(separator: " ").map(String.init) }
            .filter { $0.lowercased() != "none" }
    }

    /// What `ssh-keygen -l -F` says about a host: the keys it trusts, and
    /// the keys marked `@revoked`.
    ///
    /// The marker is only in the header before each key line
    /// (`# Host h found: line 1 REVOKED`, `… CA`), never on the key line
    /// itself (`h ED25519 SHA256:…`), so the two are read as pairs. A
    /// `@cert-authority` key is a CA's, never a host's, and is dropped. A
    /// revoked key wins over a plain line for the same key: OpenSSH refuses it.
    static func fingerprints(_ output: String) -> (trusted: Set<String>, revoked: Set<String>) {
        var trusted: Set<String> = []
        var revoked: Set<String> = []
        var marker = ""
        for line in output.components(separatedBy: .newlines) {
            if line.hasPrefix("#") {
                let words = line.split(separator: " ")
                marker = words.last.map(String.init) ?? ""
                if marker != "REVOKED", marker != "CA" { marker = "" }
                continue
            }
            guard let fingerprint = line.split(separator: " ").first(where: { $0.hasPrefix("SHA256:") }).map(String.init) else { continue }
            switch marker {
            case "REVOKED": revoked.insert(fingerprint)
            case "CA": break
            default: trusted.insert(fingerprint)
            }
            marker = ""
        }
        return (trusted.subtracting(revoked), revoked)
    }

    /// Resolve `target` with the same `ssh` the CLI runs.
    ///
    /// - Parameter ssh: the `ssh` binary; `ssh-keygen` is its sibling when it
    ///   has one.
    static func resolve(_ target: String, ssh: String) async -> Result<RemoteReach, Refusal> {
        // `--`, then the destination: a target starting with a dash would
        // otherwise be read as an option, on this Mac.
        let resolved = await ProcessRunner.run(ssh, ["-G", "--", target], deadline: 30, discardStderr: true)
        guard resolved.succeeded else { return .failure(.unresolved) }
        let settings = settings(String(decoding: resolved.stdout, as: UTF8.self))
        let destination: (host: String, port: Int, user: String)
        switch Self.destination(settings) {
        case .failure(let refusal): return .failure(refusal)
        case .success(let found): destination = found
        }

        let keygen = keygen(beside: ssh)
        let name = knownHostsName(settings, host: destination.host, port: destination.port)
        var known: Set<String> = []
        var revoked: Set<String> = []
        for file in knownHostsFiles(settings) where FileManager.default.fileExists(atPath: file) {
            let ran = await ProcessRunner.run(keygen, ["-l", "-F", name, "-f", file], deadline: 30, discardStderr: true)
            let found = fingerprints(String(decoding: ran.stdout, as: UTF8.self))
            known.formUnion(found.trusted)
            revoked.formUnion(found.revoked)
        }
        // Revoked in any file is revoked everywhere, as ssh reads it.
        known.subtract(revoked)
        guard !known.isEmpty else { return .failure(.unknownHost) }
        return .success(
            RemoteReach(host: destination.host, port: destination.port, user: destination.user, knownKeys: known, revokedKeys: revoked))
    }

    /// The `ssh` the CLI would run: the first on the CLI's `PATH`.
    static func ssh(environment: [String: String] = CLI.environment) -> String {
        let path = environment["PATH"] ?? "/usr/bin:/bin"
        for directory in path.split(separator: ":") {
            let candidate = "\(directory)/ssh"
            if FileManager.default.isExecutableFile(atPath: candidate) { return candidate }
        }
        return "/usr/bin/ssh"
    }

    private static func keygen(beside ssh: String) -> String {
        let sibling = (ssh as NSString).deletingLastPathComponent + "/ssh-keygen"
        return FileManager.default.isExecutableFile(atPath: sibling) ? sibling : "/usr/bin/ssh-keygen"
    }
}
