import Foundation
import Testing

@testable import Far_Cooler

/// What accepting a reply does on this Mac when it carries a tunneled runner.
///
/// **The rule.** A tunneled runner is usable only once its token is kept, its
/// `Host` block is in `~/.ssh/config`, and it is added under that block's
/// alias. Each of the three is its own test, and each fails against an
/// implementation that skips it.
///
/// The config is written by the real fence writer into a scratch file, so what
/// these read back is the file `ssh` would read.
struct JoinApplyTests {
    final class Keeper: TunnelRunnerKeeper, @unchecked Sendable {
        let accepts: Bool
        private let lock = NSLock()
        private var kept: [[CeremonyRunner]] = []
        init(accepts: Bool = true) { self.accepts = accepts }
        func keep(_ runners: [CeremonyRunner]) async -> Bool {
            lock.withLock { kept.append(runners) }
            return accepts
        }
        var calls: [[CeremonyRunner]] { lock.withLock { kept } }
    }

    static let identity = ShellKey.directory.appendingPathComponent("farcooler-macbook-air")
    static let program = "/Applications/Far Cooler.app/Contents/Resources/farcooler"

    private func runner(
        _ id: String, label: String = "cosmo", reach: CeremonyReach, pending: Bool = false
    ) -> CeremonyRunner {
        CeremonyRunner(
            id: id, label: label, alias: "", user: "you", host_key: "SHA256:x", reach: reach,
            pending: pending)
    }

    private func manifest(_ runners: [CeremonyRunner]) -> CeremonyManifest {
        CeremonyManifest(
            v: 2, ceremony: "c", account: "a", channel: "release", target: "t", runners: runners)
    }

    private func scratch() throws -> URL {
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("farcooler-join-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory.appendingPathComponent("config")
    }

    private func apply(
        _ runners: [CeremonyRunner], keeper: Keeper = Keeper(), config: URL,
        identity: URL? = JoinApplyTests.identity, failWrite: Bool = false
    ) async -> JoinApply.Applied {
        var join = JoinApply(keeper: keeper)
        join.cli = Self.program
        join.patternsInUse = { [] }
        join.writeConfig = { lines, identity in
            if failWrite { throw SshConfigError.io }
            try SshConfig.write(lines, identity: identity, into: config)
        }
        // The name JoinView gives a direct runner: the granter's alias, else user@host.
        return await join.run(manifest(runners), configIdentity: identity) {
            $0.alias.isEmpty ? $0.reach.name(user: $0.user) : $0.alias
        }
    }

    private func written(_ config: URL) -> String {
        (try? String(contentsOf: config, encoding: .utf8)) ?? ""
    }

    // MARK: - The path that works

    @Test func aTunneledRunnerIsKeptWrittenAndAddedUnderItsAlias() async throws {
        let config = try scratch()
        let keeper = Keeper()
        let granted = runner("you@cosmo", reach: .tailcat(token: "tc-secret"))

        let applied = await apply([granted], keeper: keeper, config: config)

        let localID = TunnelRunners.localID(for: granted)
        #expect(keeper.calls.count == 1)
        #expect(keeper.calls.first?.map(\.id) == [localID], "kept under the granter's id")
        let text = written(config)
        #expect(text.contains("Host cosmo"))
        #expect(
            text.contains("  ProxyCommand '\(Self.program)' runner pipe \(localID)"),
            "no ProxyCommand for the runner: \(text)")
        #expect(!text.contains("tc-secret"), "the token landed in ~/.ssh/config")
        #expect(!text.contains("HostName"), "a tunnel has no address to name")
        #expect(applied.targets == ["cosmo"], "added as \(applied.targets), which ssh cannot resolve")
        #expect(applied.joined.reachable.map(\.label) == ["cosmo"])
        #expect(applied.joined.note == nil)
    }

    /// The name the granter used is not this Mac's to run in a shell.
    @Test func aHostileIdNeverReachesTheProxyCommand() async throws {
        let config = try scratch()
        let ids = ["x; touch /tmp/pwned", "$(id)", "`id`", "", "-oProxyCommand=x", "a b", "a%h"]
        for id in ids {
            let granted = runner(id, label: "box \(id.count)", reach: .tailcat(token: "tc"))
            let local = TunnelRunners.localID(for: granted)
            #expect(
                local.range(of: "^fc-[0-9a-f]{16}$", options: .regularExpression) != nil,
                "\(id.debugDescription) became \(local)")
            _ = await apply([granted], config: config)
            let text = written(config)
            #expect(!text.contains("touch"), "\(id): \(text)")
            #expect(!text.contains("$(id)") && !text.contains("`id`"), "\(id): \(text)")
        }
    }

    /// Granting the same runner again replaces its entry rather than adding one.
    @Test func theSameRunnerGrantedAgainIsTheSameId() {
        let a = runner("you@cosmo", reach: .tailcat(token: "tc-1"))
        let b = runner("you@cosmo", reach: .tailcat(token: "tc-2"))
        let other = runner("you@mini", reach: .tailcat(token: "tc-1"))
        #expect(TunnelRunners.localID(for: a) == TunnelRunners.localID(for: b))
        #expect(TunnelRunners.localID(for: a) != TunnelRunners.localID(for: other))
    }

    // MARK: - The three ways it is not usable

    @Test func aTokenThatCouldNotBeKeptLeavesTheRunnerOut() async throws {
        let config = try scratch()
        let applied = await apply(
            [runner("you@cosmo", reach: .tailcat(token: "tc"))], keeper: Keeper(accepts: false),
            config: config)

        #expect(applied.targets.isEmpty, "added a runner whose token was never stored")
        #expect(applied.joined.reachable.isEmpty)
        #expect(!written(config).contains("ProxyCommand"), "wrote a block that cannot be dialed")
        let note = try #require(applied.joined.note)
        #expect(note.contains("cosmo") && note.contains("couldn’t save the tunnel"))
        #expect(applied.joined.headline == "No runners were added")
    }

    @Test func aConfigThatCouldNotBeWrittenLeavesTheTunneledRunnerOut() async throws {
        let config = try scratch()
        let applied = await apply(
            [runner("you@cosmo", reach: .tailcat(token: "tc")),
             runner("you@box", label: "box", reach: .direct(host: "box.example.com", port: 22))],
            config: config, failWrite: true)

        #expect(applied.targets == ["you@box.example.com"], "the direct runner needs no block and stays")
        let note = try #require(applied.joined.note)
        #expect(note.contains("cosmo") && note.contains("~/.ssh/config"))
        #expect(applied.configNote != nil, "the config failure went unsaid")
    }

    @Test func aMacNotWritingTheConfigCannotUseATunneledRunner() async throws {
        let config = try scratch()
        let keeper = Keeper()
        let applied = await apply(
            [runner("you@cosmo", reach: .tailcat(token: "tc"))], keeper: keeper, config: config,
            identity: nil)

        #expect(applied.targets.isEmpty)
        #expect(keeper.calls.isEmpty, "kept a token for a runner nothing can dial")
        let note = try #require(applied.joined.note)
        #expect(note.contains("Zed, Git, and Terminal turned on"))
    }

    // MARK: - What it leaves alone

    @Test func aDirectRunnerIsAddedByItsOwnNameAndNeverTouchesTheKeeper() async throws {
        let config = try scratch()
        let keeper = Keeper()
        let applied = await apply(
            [runner("you@box", label: "box", reach: .direct(host: "box.example.com", port: 22))],
            keeper: keeper, config: config)

        #expect(keeper.calls.isEmpty)
        #expect(applied.targets == ["you@box.example.com"])
        let text = written(config)
        #expect(text.contains("HostName box.example.com") && !text.contains("ProxyCommand"))
    }

    @Test func aPendingTunneledRunnerIsStillNotAddedAndNotKept() async throws {
        let config = try scratch()
        let keeper = Keeper()
        let applied = await apply(
            [runner("you@cosmo", reach: .tailcat(token: "tc"), pending: true)], keeper: keeper,
            config: config)

        #expect(applied.targets.isEmpty)
        #expect(keeper.calls.isEmpty)
        #expect(written(config).isEmpty, "wrote a block for a runner without the key")
    }

    /// Two runners lost for the same reason are one sentence naming both.
    @Test func twoRunnersLostTheSameWayAreOneSentence() async throws {
        let config = try scratch()
        let applied = await apply(
            [runner("a", label: "cosmo", reach: .tailcat(token: "t1")),
             runner("b", label: "mini", reach: .tailcat(token: "t2"))],
            keeper: Keeper(accepts: false), config: config)

        let note = try #require(applied.joined.note)
        #expect(note.contains("cosmo and mini"))
        #expect(note.components(separatedBy: "couldn’t save").count == 2)
    }

    /// Only a Mac that will write the block offers a key a tunnel would admit.
    @Test func aMacOffersTheKeyOnlyWhenItWillWriteTheConfig() {
        var choice = ShellKeyChoice(deviceName: "Test Mac")
        #expect(MacNodeIdentity.offers(shell: choice) == MacNodeIdentity.offersKey)
        choice.wanted = false
        #expect(!MacNodeIdentity.offers(shell: choice))
        choice.wanted = true
        choice.addToConfig = false
        #expect(!MacNodeIdentity.offers(shell: choice))
    }
}
