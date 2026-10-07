import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The conversation view on remote runners (ov-408): where a runner is, the
/// Mac's own key, pairing it the way a phone is, never undoing a revoke, and
/// the terminal coming back when the connection goes.
@MainActor
@Suite(.serialized)
struct RemoteConversationTests {
    // MARK: - Where a runner is

    @Test("ssh -G's answer gives the address, the port and the user")
    func sshSettingsGiveTheDestination() throws {
        let settings = RemoteReach.settings("user deploy\nhostname box.tail-1.ts.net\nport 2222\nhashknownhosts no\n")
        let found = try RemoteReach.destination(settings).get()
        #expect(found.host == "box.tail-1.ts.net")
        #expect(found.port == 2222)
        #expect(found.user == "deploy")
    }

    @Test("A runner behind ProxyJump or ProxyCommand is refused, not dialed somewhere else")
    func aProxiedRunnerIsRefused() {
        for line in ["proxyjump bastion", "proxycommand nc %h %p"] {
            let settings = RemoteReach.settings("hostname box\nport 22\n\(line)\n")
            #expect(throws: RemoteReach.Refusal.proxied) { try RemoteReach.destination(settings).get() }
        }
        let none = RemoteReach.settings("hostname box\nproxycommand none\n")
        #expect((try? RemoteReach.destination(none).get())?.host == "box")
        #expect(throws: RemoteReach.Refusal.unresolved) { try RemoteReach.destination([:]).get() }
    }

    @Test("known_hosts is asked by ssh's own name: the alias, or [host]:port off 22")
    func knownHostsNameFollowsSsh() {
        #expect(RemoteReach.knownHostsName([:], host: "box", port: 22) == "box")
        #expect(RemoteReach.knownHostsName([:], host: "127.0.0.1", port: 22411) == "[127.0.0.1]:22411")
        #expect(RemoteReach.knownHostsName(["hostkeyalias": "cosmo"], host: "10.0.0.2", port: 2222) == "cosmo")
        let files = RemoteReach.knownHostsFiles([
            "userknownhostsfile": "/u/.ssh/known_hosts /u/.ssh/known_hosts2", "globalknownhostsfile": "/etc/ssh/ssh_known_hosts",
        ])
        #expect(files == ["/u/.ssh/known_hosts", "/u/.ssh/known_hosts2", "/etc/ssh/ssh_known_hosts"])
    }

    /// `ssh-keygen -l -F h.example -f <file>`'s own output, from a file with an
    /// `@revoked` line, an `@cert-authority` line and a plain one (OpenSSH
    /// 10.3): the marker is only in the header before each key.
    static let keygenOutput = """
        # Host h.example found: line 1 REVOKED
        h.example ED25519 SHA256:sIfROYC3ydUqn1k0ucj3nnDnnJWK72iUym46jBLg22I
        # Host h.example found: line 2 CA
        h.example ED25519 SHA256:2LBnHreiGEifbAlGIPIfKnaaefiRqv9y9Mq5sxvWZek
        # Host h.example found: line 3 
        h.example ED25519 SHA256:lNiNLWZjNbE6UioQ5mbhuVEfc1ux1J3JRhnWU3u2wf4
        """

    @Test("ssh-keygen -l -F's lines give the trusted keys; @revoked ones are denied and CA keys dropped")
    func keygenLinesGiveFingerprints() {
        let found = RemoteReach.fingerprints(Self.keygenOutput)
        #expect(found.trusted == ["SHA256:lNiNLWZjNbE6UioQ5mbhuVEfc1ux1J3JRhnWU3u2wf4"])
        #expect(found.revoked == ["SHA256:sIfROYC3ydUqn1k0ucj3nnDnnJWK72iUym46jBLg22I"])
        // Revoked wins over a plain line for the same key, as in OpenSSH.
        let both = RemoteReach.fingerprints(
            "# Host h found: line 1 \nh ED25519 SHA256:x\n# Host h found: line 2 REVOKED\nh ED25519 SHA256:x\n")
        #expect(both.trusted.isEmpty && both.revoked == ["SHA256:x"])
        #expect(RemoteReach.fingerprints("").trusted.isEmpty)
    }

    /// The real `ssh -G` and `ssh-keygen` against a config and known_hosts of
    /// the test's own (through an `ssh` wrapper adding `-F`): no network.
    @Test("Resolving runs ssh -G and ssh-keygen for real, and a revoked host key isn't trusted")
    func resolveRunsTheRealTools() async throws {
        let dir = "/tmp/fc-t/rr-\(UUID().uuidString.prefix(6))"
        defer { try? FileManager.default.removeItem(atPath: dir) }
        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        var blobs: [String] = []
        for name in ["good", "bad"] {
            _ = await ProcessRunner.run("/usr/bin/ssh-keygen", ["-q", "-t", "ed25519", "-N", "", "-f", "\(dir)/\(name)"], deadline: 30)
            let line = try String(contentsOfFile: "\(dir)/\(name).pub", encoding: .utf8)
            blobs.append(String(line.split(separator: " ")[1]))
        }
        let (good, bad) = (blobs[0], blobs[1])
        try "[127.0.0.1]:2222 ssh-ed25519 \(good)\n@revoked [127.0.0.1]:2222 ssh-ed25519 \(bad)\n[127.0.0.1]:2222 ssh-ed25519 \(bad)\n"
            .write(toFile: dir + "/kh", atomically: true, encoding: .utf8)
        try "Host box\n  HostName 127.0.0.1\n  Port 2222\n  User deploy\n  UserKnownHostsFile \(dir)/kh\n  GlobalKnownHostsFile /dev/null\n"
            .write(toFile: dir + "/config", atomically: true, encoding: .utf8)
        try "#!/bin/sh\nexec /usr/bin/ssh -F \(dir)/config \"$@\"\n".write(toFile: dir + "/ssh", atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: dir + "/ssh")
        let reach = try await RemoteReach.resolve("box", ssh: dir + "/ssh").get()
        #expect(reach.host == "127.0.0.1" && reach.port == 2222 && reach.user == "deploy")
        let goodPrint = try #require(RunnerFacts.fingerprint(ofOpenSSHKey: "ssh-ed25519 \(good)"))
        let badPrint = try #require(RunnerFacts.fingerprint(ofOpenSSHKey: "ssh-ed25519 \(bad)"))
        #expect(reach.knownKeys == [goodPrint])
        #expect(reach.revokedKeys == [badPrint])
    }

    // MARK: - The key

    @Test("One key, made once and kept; its public half and client id derived from it")
    func theKeyIsMadeOnce() throws {
        let vault = MemoryConversationKeyVault()
        let key = ConversationKey(vault: vault)
        let first = try #require(key.privateKey())
        #expect(key.privateKey() == first)
        #expect(vault.writes == 1, "a second key was written")
        let publicKey = try #require(key.publicKey)
        #expect(publicKey.hasPrefix("ssh-ed25519 "))
        #expect(key.clientID == DeviceKey.clientID(of: publicKey))
        // A Mac with another key is another device.
        #expect(ConversationKey(vault: MemoryConversationKeyVault()).clientID != key.clientID)
    }

    @Test("A vault that won't keep the key gives no key, rather than a new one each time")
    func anUnkeptKeyIsNoKey() {
        struct Refusing: ConversationKeyVault {
            func read() -> ConversationKeyRead { .absent }
            func write(_ privateKey: String) -> Bool { false }
            func delete() {}
        }
        #expect(ConversationKey(vault: Refusing()).privateKey() == nil)
    }

    /// The real Keychain, under a throwaway service, deleted after. Opt-in:
    /// CI's keychain may be locked, and a normal run writes nothing there.
    @Test(
        "The Keychain keeps the key across reads",
        .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_KEYCHAIN_TESTS"] == "1"))
    func theKeychainKeepsTheKey() throws {
        var vault = KeychainConversationKeyVault()
        vault.service = "com.farcooler.conversation-key.test-\(UUID().uuidString)"
        defer { vault.delete() }
        let key = ConversationKey(vault: vault)
        let made = try #require(key.privateKey())
        #expect(vault.read() == .found(made))
        #expect(ConversationKey(vault: vault).privateKey() == made)
    }

    // MARK: - Pairing

    nonisolated static let known = "SHA256:known"
    nonisolated static let reach = RemoteReach(host: "box", port: 22, user: "me", knownKeys: [known])

    /// A runner that holds a set of keys, shows `shown` as its host key, and
    /// takes `client enroll` into that set.
    final class Runner {
        var shown = RemoteConversationTests.known
        var keys: Set<String> = []
        var offered: Set<String> = ["agent_rows", "agent_compose", "compose", "terminal_interrupt"]
        var calls: [[String]] = []
        /// Each dial, with the host key it required.
        var dials: [String?] = []
        var reachable = true
    }

    static func pairing(_ runner: Runner, vault: MemoryConversationKeyVault = MemoryConversationKeyVault(), defaults: UserDefaults)
        -> RemotePairing
    {
        let key = ConversationKey(vault: vault)
        let pairing = RemotePairing(key: key, defaults: defaults)
        pairing.resolve = { _ in .success(Self.reach) }
        pairing.cli = { args in
            runner.calls.append(args)
            if let at = args.firstIndex(of: "enroll"), let keyAt = args.firstIndex(of: "--key"), keyAt > at {
                runner.keys.insert(args[keyAt + 1])
            }
            if args.contains("revoke") { runner.keys = [] }
            return CLI.Result(ok: true, output: "{}", errors: "")
        }
        pairing.dial = { _, privateKey, pin in
            runner.dials.append(pin)
            guard runner.reachable else { throw RunnerCore.Failure.unconnected("unreachable", trouble: "unreachable", fingerprint: nil) }
            guard let pin else {
                throw RunnerCore.Failure.unconnected("unknown", trouble: "host_key_unknown", fingerprint: runner.shown)
            }
            guard pin == runner.shown else {
                throw RunnerCore.Failure.unconnected("changed", trouble: "host_key_changed", fingerprint: runner.shown)
            }
            guard let publicKey = DeviceKey.publicKey(of: privateKey), runner.keys.contains(publicKey) else {
                throw RunnerCore.Failure.unconnected("rejected", trouble: "key_rejected", fingerprint: nil)
            }
            return (RunnerCore(), runner.offered)
        }
        return pairing
    }

    static func defaults() -> UserDefaults {
        let name = "remote-conversation-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    @Test("A runner this Mac never paired gets its key the way a phone does: a control line, no shell")
    func aNewRunnerIsPairedLikeAPhone() async throws {
        let runner = Runner()
        let pairing = Self.pairing(runner, defaults: Self.defaults())
        guard case .connected(_, let offered) = await pairing.connect(target: "me@box") else {
            Issue.record("not connected: \(String(describing: pairing.states["me@box"]))")
            return
        }
        #expect(offered.contains("agent_rows"))
        #expect(pairing.states["me@box"] == .paired)
        let enroll = try #require(runner.calls.first { $0.contains("enroll") })
        #expect(enroll.starts(with: ["--json", "--runner", "me@box", "client", "enroll"]))
        #expect(enroll.contains("--scope") && enroll[enroll.firstIndex(of: "--scope")! + 1] == "control")
        #expect(!enroll.contains("--shell-access"), "the Mac's conversation key asked for a shell")
        #expect(!enroll.contains("--node-key"))
        #expect(enroll[enroll.firstIndex(of: "--client-id")! + 1] == pairing.key.clientID)
        // The probe never signs in: the first dial is unpinned, and every
        // later one requires the host key known_hosts holds.
        #expect(runner.dials.first == .some(nil))
        #expect(runner.dials.dropFirst().allSatisfy { $0 == Self.known })
    }

    @Test("A host key known_hosts doesn't hold for the runner is never signed in to")
    func anUnknownHostKeyIsRefused() async {
        let runner = Runner()
        runner.shown = "SHA256:someone-else"
        let pairing = Self.pairing(runner, defaults: Self.defaults())
        guard case .unavailable = await pairing.connect(target: "me@box") else {
            Issue.record("connected to a host ssh doesn't know")
            return
        }
        #expect(runner.dials == [nil], "dialed again after the key didn't match: \(runner.dials)")
        #expect(runner.calls.isEmpty, "paired with a host ssh doesn't know")
        guard case .unavailable(let sentence)? = pairing.states["me@box"] else {
            Issue.record("no sentence")
            return
        }
        #expect(sentence.contains("host key"))
    }

    @Test("A runner ssh reaches through a proxy says so, and pairs nothing")
    func aProxiedRunnerSaysSo() async {
        let runner = Runner()
        let pairing = Self.pairing(runner, defaults: Self.defaults())
        pairing.resolve = { _ in .failure(.proxied) }
        guard case .unavailable = await pairing.connect(target: "me@box") else {
            Issue.record("a proxied runner was dialed")
            return
        }
        #expect(runner.dials.isEmpty)
        #expect(pairing.states["me@box"] == .unavailable(RemoteReach.Refusal.proxied.sentence))
    }

    @Test("A key removed on the runner is never put back by itself, only by Pair Again")
    func aRevokeIsNeverUndone() async {
        let runner = Runner()
        let defaults = Self.defaults()
        let vault = MemoryConversationKeyVault()
        let pairing = Self.pairing(runner, vault: vault, defaults: defaults)
        guard case .connected = await pairing.connect(target: "me@box") else {
            Issue.record("never paired")
            return
        }

        // `farcooler client revoke`, on the runner.
        runner.keys = []
        runner.calls = []
        guard case .unavailable = await pairing.connect(target: "me@box") else {
            Issue.record("connected without a key")
            return
        }
        #expect(pairing.states["me@box"] == .removed)
        #expect(runner.calls.isEmpty, "re-paired a revoked key: \(runner.calls)")

        // A relaunch remembers it.
        let relaunched = Self.pairing(runner, vault: vault, defaults: defaults)
        #expect(relaunched.states["me@box"] == .removed)
        guard case .unavailable = await relaunched.connect(target: "me@box") else {
            Issue.record("re-paired after a relaunch")
            return
        }
        #expect(runner.calls.isEmpty)

        relaunched.allowPairing(target: "me@box")
        guard case .connected = await relaunched.connect(target: "me@box") else {
            Issue.record("Pair Again didn't pair")
            return
        }
        #expect(runner.calls.contains { $0.contains("enroll") })
    }

    @Test("Unpair revokes this Mac's key on the runner and keeps it unpaired")
    func unpairRevokesAndStays() async throws {
        let runner = Runner()
        let pairing = Self.pairing(runner, defaults: Self.defaults())
        guard case .connected = await pairing.connect(target: "me@box") else {
            Issue.record("never paired")
            return
        }
        #expect(await pairing.unpair(target: "me@box") == nil)
        let revoke = try #require(runner.calls.last)
        #expect(revoke == ["--json", "--runner", "me@box", "client", "revoke", pairing.key.clientID!])
        runner.calls = []
        guard case .unavailable = await pairing.connect(target: "me@box") else {
            Issue.record("connected after Unpair")
            return
        }
        #expect(runner.calls.isEmpty, "paired again after Unpair")
        #expect(pairing.states["me@box"] == .unpaired)
    }

    @Test("A runner whose hello serves no rows has its projector turned on once, then a fresh hello")
    func theProjectorIsTurnedOnOnce() async {
        let runner = Runner()
        runner.offered = []
        let pairing = Self.pairing(runner, defaults: Self.defaults())
        pairing.cli = { args in
            runner.calls.append(args)
            if args.contains("enroll"), let at = args.firstIndex(of: "--key") { runner.keys.insert(args[at + 1]) }
            if args.contains("set-projector") { runner.offered = ["agent_rows", "agent_compose"] }
            return CLI.Result(ok: true, output: "", errors: "")
        }
        guard case .connected(_, let offered) = await pairing.connect(target: "me@box") else {
            Issue.record("not connected")
            return
        }
        #expect(NativeAgents.serves(offered))
        #expect(runner.calls.contains(["--runner", "me@box", "settings", "set-projector", "on"]))
    }

    @Test("A Keychain that refuses a read is not an empty one: no new key, nothing written")
    func aFailedReadMakesNoKey() {
        let vault = MemoryConversationKeyVault("-----BEGIN OPENSSH PRIVATE KEY-----")
        vault.failing = true
        #expect(ConversationKey(vault: vault).privateKey() == nil)
        #expect(vault.writes == 0, "a refused read made a new key")
    }

    @Test("A host key known_hosts marks @revoked is never pinned or signed in to")
    func aRevokedHostKeyIsRefused() async {
        let runner = Runner()
        let pairing = Self.pairing(runner, defaults: Self.defaults())
        pairing.resolve = { _ in .success(RemoteReach(host: "box", port: 22, user: "me", knownKeys: [Self.known], revokedKeys: [Self.known])) }
        guard case .unavailable = await pairing.connect(target: "me@box") else {
            Issue.record("signed in to a revoked host key")
            return
        }
        #expect(runner.dials == [nil], "dialed with a revoked host key pinned: \(runner.dials)")
        #expect(runner.calls.isEmpty)
    }

    @Test("Unpair while a connect is on its way: the connect writes no line and no record")
    func unpairWinsTheRace() async throws {
        let runner = Runner()
        let pairing = Self.pairing(runner, defaults: Self.defaults())
        var held = true
        let dial = pairing.dial
        pairing.dial = { reach, key, pin in
            // The pinned dial waits, as a slow ssh handshake does.
            if pin != nil { while held { try await Task.sleep(for: .milliseconds(5)) } }
            return try await dial(reach, key, pin)
        }
        let connecting = Task { await pairing.connect(target: "me@box") }
        try await NativeDefaultOnTests.until { runner.dials.count >= 1 && held }
        try await Task.sleep(for: .milliseconds(50))
        #expect(await pairing.unpair(target: "me@box") == nil)
        held = false
        let outcome = await connecting.value
        if case .connected = outcome { Issue.record("connected after Unpair") }
        #expect(!runner.calls.contains { $0.contains("enroll") }, "enrolled after Unpair: \(runner.calls)")
        #expect(runner.keys.isEmpty, "a line is on the runner after Unpair")
        #expect(pairing.states["me@box"] == .unpaired)
    }

    @Test("A removed runner says so even when ssh can't resolve it, and Try Again never re-pairs it")
    func tryAgainNeverRepairs() async throws {
        let runner = Runner()
        let agents = Self.agents(runner)
        agents.start(target: "me@box")
        let terminal = try NativeAgentTests.terminal()
        try await NativeDefaultOnTests.until { agents.offers(terminal, target: "me@box") }
        runner.keys = []
        agents.pairing.resolve = { _ in .failure(.unresolved) }
        agents.retry("me@box")
        try await NativeDefaultOnTests.until { agents.pairing.states["me@box"] != .paired }
        // Not yet seen removed: resolve fails first. The record says paired.
        agents.pairing.resolve = { _ in .success(Self.reach) }
        agents.retry("me@box")
        try await NativeDefaultOnTests.until { agents.pairing.states["me@box"] == .removed }
        #expect(agents.pairing.states["me@box"] == .removed)
        runner.calls = []
        agents.pairing.resolve = { _ in .failure(.unresolved) }
        agents.retry("me@box")
        try await Task.sleep(for: .milliseconds(200))
        #expect(agents.pairing.states["me@box"] == .removed, "a passing failure hid the revoke behind Try Again")
        agents.pairing.resolve = { _ in .success(Self.reach) }
        agents.retry("me@box")
        try await Task.sleep(for: .milliseconds(200))
        #expect(runner.calls.isEmpty, "Try Again re-paired a revoked key: \(runner.calls)")
        #expect(!agents.offers(terminal, target: "me@box"))
    }

    @Test("The setting and Devices say the key is added to every runner and runs commands there")
    func theCopySaysWhatTheKeyCanDo() {
        for copy in [ConversationPairingSection.footer, NativeAgents.settingNote] {
            #expect(copy.contains("key of its own"), "\(copy)")
            #expect(copy.contains("run commands") && copy.contains("terminals"), "\(copy)")
            #expect(!copy.contains("can’t open a shell"), "\(copy)")
        }
    }

    @Test("A line the post-enroll connect didn't prove isn't recorded as paired")
    func anUnprovenLineIsNotRecorded() async {
        let runner = Runner()
        let defaults = Self.defaults()
        let pairing = Self.pairing(runner, defaults: defaults)
        let cli = pairing.cli
        pairing.cli = { args in
            let said = await cli(args)
            if args.contains("enroll") { runner.reachable = false }
            return said
        }
        guard case .unreachable = await pairing.connect(target: "me@box") else {
            Issue.record("not unreachable")
            return
        }
        #expect(defaults.dictionary(forKey: RemotePairing.recordKey)?["me@box"] == nil)
    }

    // MARK: - The view on a remote runner

    static func agents(_ runner: Runner) -> NativeAgents {
        let defaults = Self.defaults()
        defaults.set(true, forKey: NativeAgents.settingKey)
        return NativeAgents(defaults: defaults, pairing: Self.pairing(runner, defaults: defaults))
    }

    @Test("A remote runner's Claude panes are offered the view once its connection is up, and only that runner's")
    func aRemoteRunnerIsOffered() async throws {
        let runner = Runner()
        let agents = Self.agents(runner)
        let terminal = try NativeAgentTests.terminal()
        #expect(!agents.offers(terminal, target: "me@box"))
        agents.start(target: "me@box")
        try await NativeDefaultOnTests.until { agents.offers(terminal, target: "me@box") }
        #expect(agents.offers(terminal, target: "me@box"))
        #expect(!agents.offers(terminal, target: "other@box"))
        #expect(!agents.offers(terminal, target: ""), "this Mac's runner was never connected")
        let model = agents.model(for: terminal.id, target: "me@box")
        #expect(model.sink != nil)
        #expect(model.keys != nil, "Stop and Send Now aren't offered")
        #expect(model.rich)
    }

    @Test("Losing a remote runner falls back to the terminal, never a blank pane")
    func losingTheRunnerShowsTheTerminal() async throws {
        let runner = Runner()
        let agents = Self.agents(runner)
        agents.keepThroughFailures = 2
        agents.retryDelay = .milliseconds(1)
        let terminal = try NativeAgentTests.terminal(id: "0199aaaa-0000-7000-8000-0000000000a1")
        defer { NativePaneModel.remember(false, for: terminal.id) }
        agents.start(target: "me@box")
        try await NativeDefaultOnTests.until { agents.offers(terminal, target: "me@box") }

        // The pane shows the conversation; its core is never connected, so
        // its first call finds the link gone, as a dropped ssh session does.
        runner.reachable = false
        let model = agents.model(for: terminal.id, target: "me@box")
        model.setOnScreen(true, by: UUID())
        model.showsNative = true
        defer { model.store.stop() }

        try await NativeDefaultOnTests.until { !agents.offers(terminal, target: "me@box") }
        #expect(!agents.offers(terminal, target: "me@box"), "a lost runner kept its view")
        #expect(model.source == nil && model.sink == nil, "the pane kept a dead connection")
        let reconnects = runner.dials.count
        #expect(reconnects > 1, "nothing reconnected after the loss")

        // Back: the fleet's own reconnect brings the view back.
        runner.reachable = true
        agents.start(target: "me@box")
        try await NativeDefaultOnTests.until { agents.offers(terminal, target: "me@box") }
        #expect(agents.offers(terminal, target: "me@box"))
    }

    @Test("A runner removed from the fleet drops its view")
    func aForgottenRunnerOffersNothing() async throws {
        let runner = Runner()
        let agents = Self.agents(runner)
        let terminal = try NativeAgentTests.terminal()
        agents.start(target: "me@box")
        try await NativeDefaultOnTests.until { agents.offers(terminal, target: "me@box") }
        agents.forget("me@box")
        #expect(!agents.offers(terminal, target: "me@box"))
        #expect(!agents.remotes.contains("me@box"))
    }
}
