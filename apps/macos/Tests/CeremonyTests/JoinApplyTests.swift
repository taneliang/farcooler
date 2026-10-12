import Foundation
import Testing

@testable import Far_Cooler

/// What accepting a reply does on this Mac when it carries a tunneled runner.
///
/// **The rule.** A tunneled runner is usable only once its entry is in
/// `~/.ssh/config` (added to what was there, under a name that cannot be a real
/// host), its token and host-key pin are kept, and it is listed under that
/// entry's alias. Each is its own test, and each fails against an
/// implementation that skips it.
///
/// The config is written by the real fence writer into a scratch file, so what
/// these read back is the file `ssh` would read. Nothing here touches the real
/// `~/.ssh`.
@MainActor
struct JoinApplyTests {
    final class Keeper: TunnelRunnerKeeper, @unchecked Sendable {
        let accepts: Bool
        private let lock = NSLock()
        private var kept: [[CeremonyRunner]] = []
        private var forgotten: [[String]] = []
        init(accepts: Bool = true) { self.accepts = accepts }
        func keep(_ runners: [CeremonyRunner]) async -> Bool {
            lock.withLock { kept.append(runners) }
            return accepts
        }
        func forget(_ ids: [String]) async -> Bool {
            lock.withLock { forgotten.append(ids) }
            return true
        }
        var calls: [[CeremonyRunner]] { lock.withLock { kept } }
        var forgets: [[String]] { lock.withLock { forgotten } }
    }

    final class List: RunnerList {
        var added: [String] = []
        func add(_ target: String) { added.append(target) }
    }

    final class Aliases: AliasStore {
        var remembered: [String] = []
        func remember(_ alias: String, for target: String) { remembered.append(alias) }
    }

    static let identity = ShellKey.directory.appendingPathComponent("farcooler-macbook-air")
    nonisolated static let program = SshConfig.proxyProgram("/Applications/Far Cooler.app/Contents/Resources/farcooler")

    /// A real-looking fingerprint, different for each `n`.
    nonisolated static func pin(_ n: Int) -> String {
        "SHA256:" + String(repeating: String(UnicodeScalar(UInt8(65 + n % 26))), count: 43)
    }

    private func runner(
        _ id: String, label: String = "cosmo", user: String = "you", pin: Int = 1,
        reach: CeremonyReach = .tailcat(token: "tc-secret"), pending: Bool = false
    ) -> CeremonyRunner {
        CeremonyRunner(
            id: id, label: label, alias: "", user: user, host_key: Self.pin(pin), reach: reach,
            pending: pending)
    }

    private func direct(_ label: String = "box", user: String = "you", host: String = "box.example.com")
        -> CeremonyRunner
    {
        runner("you@\(label)", label: label, user: user, pin: 9, reach: .direct(host: host, port: 22))
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

    private func join(
        _ keeper: Keeper = Keeper(), config: URL, existing: [String] = [],
        failWrite: Bool = false, tool: Bool = true
    ) -> JoinApply {
        var join = JoinApply(keeper: keeper)
        join.program = { tool ? Self.program : nil }
        join.patternsInUse = { Set(existing) }
        join.readEntries = { SshConfig.entries(in: config) }
        join.writeConfig = { lines, identity in
            if failWrite { throw SshConfigError.io }
            try SshConfig.write(lines, identity: identity, into: config)
        }
        return join
    }

    private func apply(
        _ runners: [CeremonyRunner], keeper: Keeper = Keeper(), config: URL,
        identity: URL? = JoinApplyTests.identity, failWrite: Bool = false, tool: Bool = true
    ) async -> JoinApply.Applied {
        // The name JoinView gives a direct runner: the granter's alias, else user@host.
        await join(keeper, config: config, failWrite: failWrite, tool: tool).run(
            manifest(runners), configIdentity: identity
        ) { $0.alias.isEmpty ? $0.reach.name(user: $0.user) : $0.alias }
    }

    private func written(_ config: URL) -> String {
        (try? String(contentsOf: config, encoding: .utf8)) ?? ""
    }

    // MARK: - The path that works

    @Test func aTunneledRunnerIsKeptWrittenAndAddedUnderItsAlias() async throws {
        let config = try scratch()
        let keeper = Keeper()
        let granted = runner("you@cosmo")

        let applied = await apply([granted], keeper: keeper, config: config)

        let local = TunnelRunners.localID(for: granted)
        #expect(keeper.calls.first?.map(\.id) == [local], "kept under the granter's id")
        let text = written(config)
        #expect(text.contains("Host fc-cosmo"))
        #expect(text.contains("  ProxyCommand \(Self.program) runner pipe \(local)"), "\(text)")
        #expect(text.contains("StrictHostKeyChecking yes") && !text.contains("accept-new"))
        #expect(text.contains("KnownHostsCommand \(Self.program) runner known-host \(local)"))
        #expect(text.contains("# Far Cooler wrote and manages this block."), "no header: \(text)")
        #expect(text.contains("# fc-cosmo: a runner reached through the Far Cooler tunnel"))
        #expect(!text.contains("tc-secret"), "the token landed in ~/.ssh/config")
        #expect(!text.contains("HostName"), "a tunnel has no address to name")
        #expect(applied.targets == ["fc-cosmo"], "added as \(applied.targets), which ssh cannot resolve")
        #expect(applied.joined.note == nil)
    }

    /// B1. The block is first-match-wins and at the top, so a label must never
    /// become a real host's `Host`.
    @Test func aLabelThatIsARealHostnameNeverBecomesAHostEntryForIt() async throws {
        for label in ["github.com", "gitlab.com", "*", "box.example.com"] {
            let config = try scratch()
            let applied = await apply([runner("x", label: label)], config: config)
            let text = written(config)
            let hosts = text.split(separator: "\n").filter { $0.hasPrefix("Host ") }
            #expect(hosts.count == 1 && hosts[0].hasPrefix("Host fc-"), "\(label): \(hosts)")
            #expect(!(hosts.first?.contains(".") ?? false), "\(label): a dot in \(hosts)")
            #expect(applied.targets.allSatisfy { $0.hasPrefix("fc-") && !$0.contains(".") })
        }
    }

    /// The granter's id is not what a shell is handed.
    @Test func aHostileIdNeverReachesTheProxyCommand() async throws {
        let config = try scratch()
        for id in ["x; touch /tmp/pwned", "$(id)", "`id`", "", "-oProxyCommand=x", "a b", "a%h"] {
            let granted = runner(id)
            let local = TunnelRunners.localID(for: granted)
            #expect(local.range(of: "^fc-[0-9a-f]{16}$", options: .regularExpression) != nil)
            _ = await apply([granted], config: config)
            let text = written(config)
            #expect(!text.contains("touch") && !text.contains("$(id)") && !text.contains("`id`"), "\(id)")
        }
    }

    /// S2. What the runner owns names it: its host key and its user.
    @Test func theLocalIdIsTheRunnersOwnNotTheGranters() {
        let a = runner("box", pin: 1)
        #expect(TunnelRunners.localID(for: a) == TunnelRunners.localID(for: runner("you@other-name", pin: 1)))
        #expect(TunnelRunners.localID(for: a) != TunnelRunners.localID(for: runner("box", pin: 2)),
            "two machines that call themselves box share an id")
        #expect(TunnelRunners.localID(for: a) != TunnelRunners.localID(for: runner("box", user: "bob", pin: 1)),
            "two users of one box share an id")
    }

    // MARK: - B2: a later join adds

    @Test func aLaterJoinKeepsTheRunnersOfEarlierOnes() async throws {
        let config = try scratch()
        _ = await apply([runner("a", label: "cosmo", pin: 1), direct("box")], config: config)
        _ = await apply([runner("b", label: "mini", pin: 2)], config: config)

        let text = written(config)
        #expect(text.contains("Host fc-cosmo"), "the first join's tunneled runner was dropped: \(text)")
        #expect(text.contains("Host fc-mini"))
        #expect(text.contains("Host box"), "the first join's direct runner was dropped")
    }

    @Test func aRunnerGrantedAgainReplacesItsOwnEntryAndKeepsItsName() async throws {
        let config = try scratch()
        _ = await apply([runner("a", label: "cosmo", pin: 1)], config: config)
        let second = await apply([runner("other-granter", label: "cosmo", pin: 1)], config: config)

        let hosts = written(config).split(separator: "\n").filter { $0.hasPrefix("Host ") }
        #expect(hosts.count == 1, "\(hosts)")
        #expect(second.targets == ["fc-cosmo"], "renamed against itself: \(second.targets)")
    }

    @Test func aDifferentRunnerWithTheSameLabelIsRenamedNotReplaced() async throws {
        let config = try scratch()
        _ = await apply([runner("a", label: "cosmo", pin: 1)], config: config)
        let second = await apply([runner("b", label: "cosmo", pin: 2)], config: config)

        #expect(second.targets.count == 1 && second.targets != ["fc-cosmo"])
        #expect(second.joined.reachable.count == 1)
        #expect(written(config).split(separator: "\n").filter { $0.hasPrefix("Host ") }.count == 2)
    }

    @Test func theKeyAnEntryIsReadBackWithIsTheKeyItWasWrittenWith() async throws {
        let config = try scratch()
        let tunneled = runner("a", pin: 3)
        let plain = direct()
        _ = await apply([tunneled, plain], config: config)

        let keys = Set(SshConfig.entries(in: config).map(\.key))
        #expect(keys == [
            SshConfig.key(for: TunnelRunners.localized(tunneled)), SshConfig.key(for: plain),
        ])
    }

    // MARK: - The ways it is not usable

    @Test func aTokenThatCouldNotBeKeptLeavesTheRunnerOutAndItsEntryOut() async throws {
        let config = try scratch()
        _ = await apply([runner("a", label: "mini", pin: 2)], config: config)
        let applied = await apply(
            [runner("b", label: "cosmo", pin: 1), direct("box")], keeper: Keeper(accepts: false),
            config: config)

        let text = written(config)
        #expect(!text.contains("fc-cosmo"), "left an entry that cannot be dialed: \(text)")
        #expect(text.contains("Host fc-mini"), "backing out dropped an earlier runner")
        #expect(text.contains("Host box"), "backing out dropped the direct runner")
        #expect(applied.targets == ["you@box.example.com"])
        #expect(applied.remembered.map(\.alias) == ["box"], "remembered an alias for a runner that was backed out")
        let note = try #require(applied.joined.note)
        #expect(note.contains("cosmo") && note.contains("couldn’t set up the tunnel"))
    }

    /// S7. A refused config stores no token, and the notes do not contradict.
    @Test func aConfigThatCouldNotBeWrittenStoresNothingAndPromisesNothing() async throws {
        let config = try scratch()
        let keeper = Keeper()
        let applied = await apply(
            [runner("a", label: "cosmo")], keeper: keeper, config: config, failWrite: true)

        #expect(keeper.calls.isEmpty, "stored a token for a runner that was not added")
        #expect(applied.targets.isEmpty)
        let note = try #require(applied.joined.note)
        #expect(note.contains("Cosmo") == false && note.contains("cosmo wasn’t added"))
        let config2 = try #require(applied.configNote)
        #expect(!config2.contains("The keys were enrolled"), "promised it would work later: \(config2)")
    }

    @Test func aMacNotWritingTheConfigCannotUseATunneledRunner() async throws {
        let config = try scratch()
        let keeper = Keeper()
        let applied = await apply([runner("a")], keeper: keeper, config: config, identity: nil)

        #expect(applied.targets.isEmpty && keeper.calls.isEmpty)
        let note = try #require(applied.joined.note)
        #expect(note.contains("Allow Zed, Git, and Terminal") && note.contains("Add to ~/.ssh/config"))
    }

    @Test func noCommandLineToolMeansNoTunneledRunner() async throws {
        let config = try scratch()
        let keeper = Keeper()
        let applied = await apply([runner("a")], keeper: keeper, config: config, tool: false)

        #expect(applied.targets.isEmpty && keeper.calls.isEmpty)
        #expect(!written(config).contains("ProxyCommand"))
    }

    /// R-48. Without a host key there is nothing to pin.
    @Test func aTunneledRunnerWithNoHostKeyIsLeftOut() async throws {
        let config = try scratch()
        let keeper = Keeper()
        var granted = runner("a")
        granted.host_key = ""
        let applied = await apply([granted], keeper: keeper, config: config)

        #expect(applied.targets.isEmpty && keeper.calls.isEmpty)
        #expect(try #require(applied.joined.note).contains("didn’t share its host key"))
    }

    /// S3. One space in a `User` line and every ssh on the Mac fails.
    @Test func detailsSshCouldNotParseNeverReachTheFile() async throws {
        let config = try scratch()
        let applied = await apply(
            [runner("a", user: "you extra"), direct("box", user: "ok", host: "bad host"),
             direct("fine")],
            config: config)

        let text = written(config)
        #expect(!text.contains("you extra") && !text.contains("bad host"), "\(text)")
        #expect(applied.targets == ["you@box.example.com"])
        #expect(text.contains("Host fine"))
        #expect(applied.joined.lost.count == 2)
    }

    // MARK: - What it leaves alone

    @Test func aDirectRunnerIsAddedByItsOwnNameAndNeverTouchesTheKeeper() async throws {
        let config = try scratch()
        let keeper = Keeper()
        let applied = await apply([direct()], keeper: keeper, config: config)

        #expect(keeper.calls.isEmpty)
        #expect(applied.targets == ["you@box.example.com"])
        let text = written(config)
        #expect(text.contains("HostName box.example.com") && !text.contains("  ProxyCommand"))
        #expect(text.contains("# box: a runner reached at its address"))
    }

    @Test func aPendingTunneledRunnerIsStillNotAddedAndNotKept() async throws {
        let config = try scratch()
        let keeper = Keeper()
        let applied = await apply([runner("a", pending: true)], keeper: keeper, config: config)

        #expect(applied.targets.isEmpty && keeper.calls.isEmpty)
        #expect(written(config).isEmpty, "wrote a block for a runner without the key")
    }

    /// Two runners lost for the same reason are one sentence naming both.
    @Test func twoRunnersLostTheSameWayAreOneSentence() async throws {
        let config = try scratch()
        let applied = await apply(
            [runner("a", label: "cosmo", pin: 1), runner("b", label: "mini", pin: 2)],
            keeper: Keeper(accepts: false), config: config)

        let note = try #require(applied.joined.note)
        #expect(note.contains("cosmo and mini"))
        #expect(note.components(separatedBy: "couldn’t set up").count == 2)
    }

    /// A runner with no label is named “a tunneled runner”, which cannot start a
    /// sentence in lowercase.
    @Test func aSentenceNeverStartsInLowercase() async throws {
        let config = try scratch()
        let applied = await apply(
            [runner("a", label: "", pending: true)], config: config)
        let note = try #require(applied.joined.note)
        #expect(note.hasPrefix("A tunneled runner"), Comment(rawValue: note))
    }

    // MARK: - The seam the sheet calls

    @Test func aFailedWriteAddsNoTunneledRunnerAndRemembersNoAlias() async throws {
        let config = try scratch()
        let list = List()
        let aliases = Aliases()
        let applied = await join(config: config, failWrite: true).settle(
            manifest([runner("a"), direct()]), configIdentity: Self.identity,
            target: { $0.label }, runners: list, aliases: aliases)

        #expect(list.added == ["box"], "added \(list.added)")
        #expect(aliases.remembered.isEmpty, "remembered an alias for a block never written")
        #expect(applied.targets == ["box"])
    }

    @Test func aWrittenBlockAddsTheRunnersAndRemembersTheirAliases() async throws {
        let config = try scratch()
        let list = List()
        let aliases = Aliases()
        _ = await join(config: config).settle(
            manifest([runner("a"), direct()]), configIdentity: Self.identity,
            target: { $0.label }, runners: list, aliases: aliases)

        #expect(list.added == ["fc-cosmo", "box"])
        #expect(Set(aliases.remembered) == ["fc-cosmo", "box"])
    }

    // MARK: - The offer

    private struct Vault: NodeKeyVault {
        func read() -> NodeKeyRead { .found(NodeKeyPair(privateKey: "p", publicKey: "pub")) }
        func write(_ pair: NodeKeyPair) -> Bool { true }
    }

    private struct Tool: NodeKeyTool {
        func mint() async -> NodeKeyPair? { nil }
        func keep(privateKey: String) async -> Bool { true }
    }

    /// S4. With the flag injected, so the gate can fail while the real flag is off.
    @Test func aMacOffersTheKeyOnlyWhenItWillWriteTheConfig() async {
        let identity = MacNodeIdentity(vault: Vault(), tool: Tool())
        var choice = ShellKeyChoice(deviceName: "Test Mac")
        #expect(await identity.offer(for: choice, flag: true) == "pub")
        #expect(await identity.offer(for: choice, flag: false) == nil, "the flag is not honored")
        choice.addToConfig = false
        #expect(await identity.offer(for: choice, flag: true) == nil, "offered without the config block")
        choice.addToConfig = true
        choice.wanted = false
        #expect(await identity.offer(for: choice, flag: true) == nil, "offered without shell access")
    }

    // MARK: - S1: taking one back out

    @Test func removingATunneledRunnerRemovesItsEntryAndItsToken() async throws {
        let config = try scratch()
        _ = await apply([runner("a", label: "cosmo", pin: 1), runner("b", label: "mini", pin: 2)],
            config: config)
        let keeper = Keeper()
        var forget = TunnelForget(keeper: keeper)
        forget.readEntries = { SshConfig.entries(in: config) }
        forget.writeConfig = { try SshConfig.write($0, identity: $1, into: config) }

        let removed = await forget.forget(target: "fc-cosmo")

        let text = written(config)
        #expect(removed)
        #expect(!text.contains("fc-cosmo"), "the entry outlived the runner: \(text)")
        #expect(text.contains("Host fc-mini"), "removed another runner too")
        #expect(keeper.forgets == [[TunnelRunners.localID(for: runner("a", label: "cosmo", pin: 1))]])
        #expect(await forget.forget(target: "no-such-runner") == false)
    }
}
