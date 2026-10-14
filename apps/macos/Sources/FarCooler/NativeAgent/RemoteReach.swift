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
    /// `<cli> runner pipe <id>`, when the config reaches this runner through
    /// Far Cooler's own tunnel `ProxyCommand` (ov-496). The core spawns it and
    /// speaks ssh over it, the way `ssh` would; `host` and `port` then name
    /// nothing to dial and the key it must show is the one the granting device
    /// pinned (`runner pinned-key`), not one from `known_hosts`.
    var pipe: [String]? = nil

    /// Why a runner can't be dialed by the client core. Each is a sentence for
    /// Settings, never ssh's own words.
    enum Refusal: Error, Equatable, Sendable {
        /// `ssh -G` named no host: the target isn't one ssh can resolve.
        case unresolved
        /// Reached through `ProxyJump` or any `ProxyCommand` but Far Cooler's
        /// own tunnel one, which the core can't follow.
        case proxied
        /// No key for it in `known_hosts`.
        case unknownHost
        /// A tunneled runner, and the Far Cooler command line tool its
        /// `ProxyCommand` names isn't this app's own.
        case tunnelUnavailable
        /// A tunneled runner with no host key on file from when it was added.
        case notGranted

        var sentence: String {
            switch self {
            case .unresolved: "Far Cooler couldn’t look up this runner’s address in your ssh settings."
            case .proxied: "This runner is reached through a proxy, so its Claude and Codex panes show the terminal."
            case .unknownHost: "Connect to this runner once with ssh, so Far Cooler can check its host key."
            case .tunnelUnavailable: "Far Cooler couldn’t reach this runner’s tunnel from here. Quit and reopen Far Cooler to try again."
            case .notGranted: "Far Cooler has no host key on file for this runner. Remove it and add it again."
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
    ///
    /// - Parameter cli: the program Far Cooler's tunnel `ProxyCommand` names
    ///   (``StableCLI``'s link). A `ProxyCommand` of exactly
    ///   `<cli> runner pipe <id>` is followed; any other is refused.
    static func destination(
        _ settings: [String: String], cli: String = StableCLI.url().path
    ) -> Result<(host: String, port: Int, user: String, pipe: [String]?), Refusal> {
        if let value = settings["proxyjump"], !value.isEmpty, value.lowercased() != "none" { return .failure(.proxied) }
        var pipe: [String]?
        if let value = settings["proxycommand"], !value.isEmpty, value.lowercased() != "none" {
            guard let found = runnerPipe(value, cli: cli) else { return .failure(.proxied) }
            pipe = found
        }
        guard let host = settings["hostname"], !host.isEmpty else { return .failure(.unresolved) }
        let port = Int(settings["port"] ?? "") ?? 22
        let user = settings["user"].flatMap { $0.isEmpty ? nil : $0 } ?? NSUserName()
        return .success((host, port, user, pipe))
    }

    /// `[cli, "runner", "pipe", id]` when `command` is exactly that line as
    /// ``SshConfig`` writes it (the program one shell word, single-quoted when
    /// it needs it, `%` doubled), else nil. Nothing else a config says is run.
    static func runnerPipe(_ command: String, cli: String) -> [String]? {
        guard let words = shellWords(command), words.count == 4,
            words[0] == cli, words[1] == "runner", words[2] == "pipe"
        else { return nil }
        guard plainID(words[3]) else { return nil }
        return words
    }

    /// An id that is safe as one word on a command line: ASCII letters,
    /// digits, `.`, `_` and `-`, 1 to 64 of them, not led by `-` or `.`.
    static func plainID(_ id: String) -> Bool {
        let plain = id.unicodeScalars.allSatisfy { $0.isASCII && (CharacterSet.alphanumerics.contains($0) || "._-".unicodeScalars.contains($0)) }
        return plain && !id.isEmpty && id.count <= 64 && !id.hasPrefix("-") && !id.hasPrefix(".")
    }

    /// Split `line` the way `sh` would for plain and single-quoted words,
    /// after `ssh` has turned `%%` into `%`. Nil for anything with an
    /// expansion, a quote of another kind, or an unclosed quote.
    private static func shellWords(_ line: String) -> [String]? {
        var words: [String] = []
        var word = ""
        var inWord = false
        var quoted = false
        let text = line.replacingOccurrences(of: "%%", with: "%")
        for character in text {
            if quoted {
                if character == "'" { quoted = false } else { word.append(character) }
            } else if character == "'" {
                quoted = true
                inWord = true
            } else if character == " " {
                if inWord { words.append(word) }
                word = ""
                inWord = false
            } else if "\\\"$`;&|<>(){}*?[]#~!\n\t".contains(character) || character == "%" {
                return nil
            } else {
                word.append(character)
                inWord = true
            }
        }
        guard !quoted else { return nil }
        if inWord { words.append(word) }
        return words
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
    /// - Parameter cli: the program Far Cooler's tunnel `ProxyCommand` names,
    ///   see ``destination(_:cli:)``.
    /// - Parameter bundled: this app's own CLI. A tunneled runner is reached
    ///   by running THIS, and only once `cli`'s link resolves to it: the
    ///   config's word recognizes the line, and is never what gets run.
    static func resolve(
        _ target: String, ssh: String, cli: String = StableCLI.url().path, bundled: String? = CLI.binary
    ) async -> Result<RemoteReach, Refusal> {
        // `--`, then the destination: a target starting with a dash would
        // otherwise be read as an option, on this Mac.
        let resolved = await ProcessRunner.run(ssh, ["-G", "--", target], deadline: 30, discardStderr: true)
        guard resolved.succeeded else { return .failure(.unresolved) }
        let settings = settings(String(decoding: resolved.stdout, as: UTF8.self))
        let destination: (host: String, port: Int, user: String, pipe: [String]?)
        switch Self.destination(settings, cli: cli) {
        case .failure(let refusal): return .failure(refusal)
        case .success(let found): destination = found
        }
        if let pipe = destination.pipe {
            return await pinned(destination, id: pipe[3], pipe: pipe, link: cli, bundled: bundled)
        }
        // A Tailcat runner a person moved to its address (ov-485): dialed at
        // the address, and still held to the pin the granter sent, which is
        // what its `HostKeyAlias` names. Its `known_hosts` files are `/dev/null`.
        if let id = tunnelID(settings) {
            return await pinned(destination, id: id, pipe: nil, link: cli, bundled: bundled)
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

    /// The id a Tailcat runner's pin is filed under, from `HostKeyAlias
    /// farcooler-<id>`, when it is a plain one.
    static func tunnelID(_ settings: [String: String]) -> String? {
        guard let alias = settings["hostkeyalias"], alias.hasPrefix(SshConfig.tunnelAliasPrefix) else { return nil }
        let id = String(alias.dropFirst(SshConfig.tunnelAliasPrefix.count))
        return plainID(id) ? id : nil
    }

    /// A runner reached over Far Cooler's tunnel: the key it must show is the
    /// one the granting device pinned, which the CLI holds (`runner pinned-key`).
    /// No `known_hosts`: the block points both files at `/dev/null`.
    private static func pinned(
        _ destination: (host: String, port: Int, user: String, pipe: [String]?), id: String, pipe: [String]?,
        link: String, bundled: String?
    ) async -> Result<RemoteReach, Refusal> {
        // Resolved once, and the RESOLVED path is what runs: the link is
        // writable by this user, and a check of it followed by a run of it
        // would leave a gap to swap it in.
        guard let bundled, let real = realPath(bundled), realPath(link) == real,
            FileManager.default.isExecutableFile(atPath: real)
        else { return .failure(.tunnelUnavailable) }
        let ran = await ProcessRunner.run(real, ["runner", "pinned-key", id], environment: CLI.environment, deadline: 30, discardStderr: true)
        let key = String(decoding: ran.stdout, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        guard ran.succeeded, key.hasPrefix("SHA256:"), !key.contains(" ") else { return .failure(.notGranted) }
        return .success(
            RemoteReach(
                host: destination.host, port: destination.port, user: destination.user, knownKeys: [key],
                pipe: pipe.map { _ in [real, "runner", "pipe", id] }))
    }

    private static func realPath(_ path: String) -> String? {
        guard let resolved = realpath(path, nil) else { return nil }
        defer { free(resolved) }
        return String(cString: resolved)
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
