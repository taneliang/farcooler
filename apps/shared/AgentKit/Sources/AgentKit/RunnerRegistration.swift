import Foundation

/// Putting a runner in a team and handing it a token to notify with (ov-528).
///
/// Replaces the Mac's `/v1/daemons` pairing. Four steps, in this order, and
/// the order is the design:
///
/// 1. Ask the directory who is signed in and which team to use.
/// 2. Ask the RUNNER, over the ssh channel the app already has, to sign a
///    statement naming this account, the team and the relay. The install id
///    alone proves nothing (it is in `ps`), so the relay wants the runner's own
///    key, which never leaves the box.
/// 3. Post that proof to the directory, which answers with a token.
/// 4. Pipe the token into `farcooler push pair` on the runner.
///
/// The token exists between steps 3 and 4 and is bound in one place, the
/// `.success` branch below. It goes to the runner on stdin and nowhere else:
/// not an argument (`ps` shows arguments, here and over ssh), not the result,
/// not a log line. `scrubbed` removes it from whatever the runner says back.
///
/// Moving a runner this person already registered in another of their teams is
/// the same four steps with the runner's signed claim in place of its
/// registration (`move`). It is only ever offered after the relay said
/// `elsewhere`, which it says only to someone who administers the runner, and
/// the person has confirmed that people with access through the old team lose it.
///
/// Everything outside the relay is a closure, so the tests drive the real
/// sequence with a relay that is a script and a "runner" that is a list.
@MainActor
public struct RunnerRegistration {
    /// What a `farcooler` invocation said.
    public struct CLIOutput {
        public var ok: Bool
        /// Stdout only.
        public var output: String
        /// Stdout and stderr together, for the runner's own words.
        public var said: String

        public init(ok: Bool, output: String, said: String) {
            self.ok = ok
            self.output = output
            self.said = said
        }
    }

    /// How it ended. `message` is for the person; `detail` is the runner's own
    /// words, for a disclosure, when there are any worth showing.
    public enum Outcome: Equatable {
        case registered(team: String, message: String)
        case failed(message: String, detail: String?, reason: Reason)
        /// The runner is this person's, in another of their teams. `message`
        /// is the question to ask before moving it, and `runnerKey` is the key
        /// the runner showed when it registered: the move is only for that one.
        case needsMove(message: String, runnerKey: String)

        public var message: String {
            switch self {
            case .registered(_, let message), .failed(let message, _, _), .needsMove(let message, _): return message
            }
        }

        public var succeeded: Bool {
            if case .registered = self { return true }
            return false
        }
    }

    /// Why a failure happened, where the app can do something about it.
    public enum Reason: Equatable {
        case other
        /// The Far Cooler on the runner predates `sign-registration`; installing
        /// again fixes it.
        case runnerTooOld
    }

    /// Run `farcooler` with these arguments and, if given, this on stdin.
    public typealias RunCLI = (_ arguments: [String], _ stdin: String?) async -> CLIOutput

    private let directory: DirectoryClient
    private let relay: String
    private let defaultRelay: String
    private let run: RunCLI

    /// - Parameters:
    ///   - relay: the relay this app talks to; the runner's proof names it.
    ///   - defaultRelay: this build's own, so a runner is only told a relay
    ///     when it is not the one it would use anyway.
    public init(directory: DirectoryClient, relay: String, defaultRelay: String, run: @escaping RunCLI) {
        self.directory = directory
        self.relay = relay
        self.defaultRelay = defaultRelay
        self.run = run
    }

    /// The teams a runner may join, personal first.
    public func teams() async -> Result<[DirectoryTeam], DirectoryFailure> {
        await directory.me().map(\.joinable)
    }

    /// Register the runner at `target` (`""` is this Mac) in `team` and pair it.
    ///
    /// `directoryName` is what the team sees: the computer's name or the
    /// runner's hostname, never the ssh target (which can hold a username and
    /// an address).
    public func register(target: String, team: DirectoryTeam, directoryName: String) async -> Outcome {
        await perform(target: target, team: team, directoryName: directoryName, moving: false)
    }

    /// Move a runner this person registered in another team to `team`.
    ///
    /// `runnerKey` is the one `.needsMove` carried. The claim is refused unless
    /// the runner signs with that same key, so "Move it?" is about exactly the
    /// runner the relay said was theirs, not another one that answers on the
    /// same address.
    public func move(target: String, team: DirectoryTeam, directoryName: String, runnerKey: String) async -> Outcome {
        await perform(target: target, team: team, directoryName: directoryName, moving: true, expectedKey: runnerKey)
    }

    private func perform(
        target: String, team: DirectoryTeam, directoryName: String, moving: Bool, expectedKey: String? = nil
    ) async -> Outcome {
        let name = target.isEmpty ? "This Mac" : target
        let verb = moving ? "moved" : "added"

        // 1. The account the proof must name, and a check that the team is
        // still one this person may add to.
        let me: DirectoryMe
        switch await directory.me() {
        case .failure(let why): return .failed(message: why.message, detail: nil, reason: .other)
        case .success(let found): me = found
        }
        guard me.joinable.contains(where: { $0.id == team.id }) else {
            return .failed(message: DirectoryFailure.teamUnavailable.message, detail: nil, reason: .other)
        }

        // 2. The runner signs. The relay's address is spelled out because the
        // proof names it, and a runner left to guess would sign for its own
        // build's relay.
        let host = target.isEmpty ? [] : ["--host", target]
        let signed = await run(
            host + [
                "runner", moving ? "claim" : "sign-registration", "--account", me.account, "--team", team.id,
                "--relay", Self.origin(of: relay),
            ],
            nil)
        guard signed.ok else {
            let tooOld = Self.isTooOld(signed.said)
            return .failed(
                message: Self.signFailure(tooOld: tooOld, name: name, verb: verb), detail: signed.said,
                reason: tooOld ? .runnerTooOld : .other)
        }
        guard let proof = RunnerProof(printed: signed.output) else {
            // Its words are shown: a box whose shell prints something odd is
            // the usual cause, and nothing here is secret (no token exists yet).
            return .failed(
                message: "\(name) wasn’t \(verb) because its answer didn’t make sense. "
                    + "Check that its login shell prints nothing of its own, then try again.",
                detail: signed.said, reason: .other)
        }

        if let expectedKey, proof.runnerKey != expectedKey {
            return .failed(
                message: "\(name) wasn’t \(verb) because it answered as a different runner.", detail: nil,
                reason: .other)
        }

        // 3. The directory registers it, or moves it.
        let answer = moving
            ? await directory.claim(team: team.id, name: directoryName, proof: proof)
            : await directory.register(team: team.id, name: directoryName, proof: proof)
        switch answer {
        case .failure(.inAnotherTeam) where !moving:
            return .needsMove(
                message: "\(name) is already in another of your teams. Move it to \(team.name)? "
                    + "Anyone else with access to it loses that access.",
                runnerKey: proof.runnerKey)
        case .failure(let why):
            return .failed(message: why.message, detail: nil, reason: .other)

        case .success(let token):
            // 4. Straight down the pipe. Nothing between the line above and
            // this one, and nothing after it keeps `token`.
            var pair = host + ["push", "pair"]
            if Self.origin(of: relay) != Self.origin(of: defaultRelay) {
                pair += ["--relay", Self.trimmed(relay)]
            }
            let paired = await run(pair, token)
            guard paired.ok else {
                // The relay has rotated the token already, so a runner that was
                // notifying has stopped. Say so.
                return .failed(
                    message: "\(name) is in \(team.name), but it couldn’t finish setting up notifications, "
                        + "so it won’t notify your devices until you try again.",
                    detail: Self.scrubbed(paired.said, token: token), reason: .other)
            }
            return .registered(
                team: team.name,
                message: moving
                    ? "\(name) moved to \(team.name) and can notify your devices."
                    : "\(name) is in \(team.name) and can now notify your devices.")
        }
    }

    // MARK: - Words

    /// Whether a runner's refusal is the old CLI not knowing the command: clap's
    /// own words, which arrive through ssh's inherited stderr.
    static func isTooOld(_ said: String) -> Bool {
        said.contains("unrecognized subcommand") || said.contains("unexpected argument")
    }

    /// Why the runner wouldn't sign, as far as its words tell.
    public static func signFailure(tooOld: Bool, name: String, verb: String) -> String {
        if tooOld {
            return "\(name) wasn’t \(verb) because the Far Cooler on it is too old to prove it’s yours. "
                + "Install Far Cooler on it again, then retry."
        }
        return "\(name) wasn’t \(verb) because it couldn’t prove it’s yours. "
            + "Check that you can reach it over SSH and that Far Cooler is installed on it."
    }

    /// `text` with every occurrence of the token taken out.
    ///
    /// The token is never in a command line, so a runner has nothing to echo
    /// back. This is for the day one does anyway.
    static func scrubbed(_ text: String, token: String) -> String {
        guard !token.isEmpty else { return text }
        return text.replacingOccurrences(of: token, with: "…")
    }

    /// `scheme://host[:port]` of an address, spelled the way the relay spells
    /// its own origin: scheme and host in lowercase, and no port when it's the
    /// scheme's default. The relay compares the proof against that, so a typed
    /// `HTTPS://Relay.Example:443/x` has to come out as `https://relay.example`.
    static func origin(of address: String) -> String {
        guard let url = URL(string: address), let scheme = url.scheme?.lowercased(), let host = url.host?.lowercased()
        else { return address }
        let isDefault = (scheme == "https" && url.port == 443) || (scheme == "http" && url.port == 80)
        let port = url.port.flatMap { isDefault ? nil : ":\($0)" } ?? ""
        return "\(scheme)://\(host.contains(":") ? "[\(host)]" : host)\(port)"
    }

    /// The address as configured, without a trailing slash: what `push pair
    /// --relay` stores, so a relay under a path prefix keeps its prefix.
    static func trimmed(_ address: String) -> String {
        var out = address.trimmingCharacters(in: .whitespaces)
        while out.hasSuffix("/") { out.removeLast() }
        return out
    }
}
