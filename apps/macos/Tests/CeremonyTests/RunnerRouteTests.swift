import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The addresses a test keeps, in memory rather than in anybody's defaults.
final class MemoryRouteAlternatives: RouteAlternatives, @unchecked Sendable {
    private let lock = NSLock()
    private var kept: [String: (host: String, port: Int)] = [:]

    func address(for id: String) -> (host: String, port: Int)? { lock.withLock { kept[id] } }
    func remember(host: String, port: Int, for id: String) { lock.withLock { kept[id] = (host, port) } }
    func forget(_ id: String) { lock.withLock { kept[id] = nil } }
}

/// The owner's route rule on the Mac (ov-485): a runner granted through
/// Tailcat keeps its address beside it, takes one route at a time, and moves
/// to the address only when a person answers the question.
@MainActor
struct RunnerRouteTests {
    let alternatives = MemoryRouteAlternatives()

    // MARK: - Joining keeps the address

    /// A tunneled runner with an address beside it: kept on the tunnel, and
    /// the address remembered under the id its `ProxyCommand` names.
    @Test func joiningKeepsTheAddressBesideTheTunnel() async throws {
        let config = try scratch()
        _ = await join(config).run(
            manifest([granted("cosmo", host: "192.168.1.180")]), configIdentity: JoinApplyTests.identity
        ) { $0.alias }

        let id = TunnelRunners.localID(for: granted("cosmo", host: "192.168.1.180"))
        #expect(alternatives.address(for: id)?.host == "192.168.1.180")
        let routes = switcher(config)
        #expect(routes.route(of: "fc-cosmo") == .tunnel)
        #expect(routes.alternative(of: "fc-cosmo") == "192.168.1.180")
    }

    /// An alternative that isn't one host word on a real port is never kept,
    /// so it can never reach `~/.ssh/config` (M1).
    @Test func aMisshapenAlternativeIsNeverKept() async throws {
        for (host, port) in [("a b", 22), ("box", 0), ("box", 65536), ("box\nProxyCommand x", 22), ("-oProxy", 22)] {
            let config = try scratch()
            var runner = granted("cosmo", host: nil)
            runner.alternative = .direct(host: host, port: port)
            _ = await join(config).run(manifest([runner]), configIdentity: JoinApplyTests.identity) { $0.alias }
            #expect(alternatives.address(for: TunnelRunners.localID(for: runner)) == nil, "kept \(host):\(port)")
            #expect(switcher(config).route(of: "fc-cosmo") == nil)
        }
    }

    /// Granted again, a runner a person moved to its address stays there
    /// (L3): Connect Directly sticks until Use Tailcat.
    @Test func grantingAgainKeepsConnectDirectly() async throws {
        let config = try await joined()
        #expect(switcher(config).useAddress(for: "fc-cosmo"))
        _ = await join(config).run(
            manifest([granted("cosmo", host: "192.168.1.180")]), configIdentity: JoinApplyTests.identity
        ) { $0.alias }
        #expect(switcher(config).route(of: "fc-cosmo") == .direct(address: "192.168.1.180"))
        #expect(!written(config).contains("  ProxyCommand "))
    }

    // MARK: - Moving between routes

    /// Connect Directly writes the address where the `ProxyCommand` was, and
    /// keeps the pin: the address is held to the key the granter pinned (R-48).
    @Test func theAddressRouteKeepsThePin() async throws {
        let config = try await joined()
        let routes = switcher(config)

        #expect(routes.useAddress(for: "fc-cosmo"))

        let text = written(config)
        #expect(text.contains("HostName 192.168.1.180"), "\(text)")
        #expect(!text.contains("  ProxyCommand "), "two routes at once: \(text)")
        #expect(text.contains("HostKeyAlias farcooler-"), "the pin went: \(text)")
        #expect(text.contains("KnownHostsCommand"), "the pin went: \(text)")
        #expect(routes.route(of: "fc-cosmo") == .direct(address: "192.168.1.180"))
    }

    /// And Use Tailcat puts it back as it was written at the grant.
    @Test func useTailcatPutsTheTunnelBack() async throws {
        let config = try await joined()
        let before = written(config)
        let routes = switcher(config)

        routes.useAddress(for: "fc-cosmo")
        #expect(routes.useTunnel(for: "fc-cosmo"))

        #expect(written(config) == before)
        #expect(routes.route(of: "fc-cosmo") == .tunnel)
    }

    /// A runner with no address kept has nothing to switch to.
    @Test func noAddressKeptMeansNoChoice() async throws {
        let config = try await joined(host: nil)
        let routes = switcher(config)
        #expect(routes.route(of: "fc-cosmo") == nil)
        #expect(!routes.useAddress(for: "fc-cosmo"))
        #expect(written(config).contains("  ProxyCommand "))
    }

    // MARK: - The question

    /// What ssh passes through when the tunnel never reached the runner:
    /// the pipe's sentence, its marker line, and ssh's own closing words.
    static let unreached = """
        farcooler: the runner did not answer. It may be off, or this device's access to it may have been revoked.
        farcooler-route: tunnel-unreached
        Connection closed by UNKNOWN port 65535
        """

    private func prompt(_ config: URL) -> RoutePrompt {
        let prompt = RoutePrompt()
        prompt.routes = switcher(config)
        prompt.gathering = 0
        prompt.reconnect = { _ in Issue.record("dialed without being asked") }
        prompt.tell = { Issue.record("told: \($0)") }
        return prompt
    }

    /// Polls, never sleeps a fixed time: the question is put up from its own
    /// task, after the store update that noted the failure.
    private func eventually(_ condition: () -> Bool) async {
        let deadline = Date().addingTimeInterval(30)
        while !condition(), Date() < deadline { try? await Task.sleep(for: .milliseconds(10)) }
    }

    /// Tailcat never reached the runner: the owner's question is asked once,
    /// with what choosing it does, and nothing moves until it is answered.
    @Test func aFailedTunnelAsksOnceAndMovesNothingByItself() async throws {
        let config = try await joined()
        let prompt = prompt(config)
        var asked: [RouteQuestion] = []
        prompt.ask = { question, _ in asked.append(question) }

        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: Self.unreached))
        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: Self.unreached))
        #expect(asked.isEmpty, "asked inside the store update")
        await eventually { !asked.isEmpty }

        #expect(asked.map(\.runners) == [[.init(target: "fc-cosmo", address: "192.168.1.180")]])
        #expect(asked.first?.title == "Connecting through Tailcat failed.")
        #expect(asked.first?.message.hasPrefix("Connect directly to 192.168.1.180 instead?") == true)
        #expect(asked.first?.message.contains(DirectRouteOffer.afterwards) == true, "doesn't say it sticks")
        #expect(written(config).contains("  ProxyCommand "), "moved to the address on its own")
    }

    /// Connect Directly moves the runner and dials it again on its address.
    @Test func connectDirectlyMovesTheRunnerAndRedials() async throws {
        let config = try await joined()
        let prompt = prompt(config)
        prompt.ask = { _, answer in answer(true) }
        var redialed: [String] = []
        prompt.reconnect = { redialed.append($0) }

        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: Self.unreached))
        await eventually { !redialed.isEmpty }

        #expect(redialed == ["fc-cosmo"])
        #expect(written(config).contains("HostName 192.168.1.180"))
    }

    /// A move the config refused is said, in a sentence, not dropped (L5).
    @Test func aMoveThatCouldNotBeWrittenIsSaid() async throws {
        let config = try await joined()
        let prompt = prompt(config)
        prompt.routes.writeConfig = { _, _ in throw SshConfigError.io }
        prompt.ask = { _, answer in answer(true) }
        var told: [String] = []
        prompt.tell = { told.append($0) }

        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: Self.unreached))
        await eventually { !told.isEmpty }

        #expect(told == [RoutePrompt.couldNotMove("fc-cosmo")])
    }

    /// Not Now leaves it on Tailcat, and it is asked again only after the
    /// runner has answered once.
    @Test func notNowLeavesItOnTailcatUntilItAnswersAgain() async throws {
        let config = try await joined()
        let prompt = prompt(config)
        var asks = 0
        prompt.ask = { _, answer in asks += 1; answer(false) }

        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: Self.unreached))
        await eventually { asks == 1 }
        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: Self.unreached))
        prompt.heard(target: "fc-cosmo", state: .connected)
        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: Self.unreached))
        await eventually { asks == 2 }

        #expect(asks == 2)
        #expect(written(config).contains("  ProxyCommand "))
    }

    /// Never asked when anything on the runner answered (M2): a refused key,
    /// a host key that isn't the pinned one, the CLI's known-host refusal, an
    /// old build. Nor about this Mac, a runner not installed, or one already
    /// on its address.
    @Test func nothingIsAskedWhereTheAddressCannotHelp() async throws {
        let config = try await joined()
        let prompt = prompt(config)
        var asks = 0
        prompt.ask = { _, _ in asks += 1 }

        for reason in [
            "you@cosmo: Permission denied (publickey).\nCould not reach fc-cosmo over ssh.",
            "Host key verification failed.\nCould not reach fc-cosmo over ssh.",
            "farcooler: SHA256:abc is not the host key pinned for fc-x\nHost key verification failed.",
            "fc-cosmo runs protocol 3; this client speaks 4.\nUpdate the older side:  farcooler runner install fc-cosmo",
            "Couldn’t reach this runner.",
        ] {
            prompt.heard(target: "fc-cosmo", state: .unreachable(reason: reason))
        }
        prompt.heard(target: "", state: .unreachable(reason: Self.unreached))
        prompt.heard(target: "fc-cosmo", state: .notInstalled)
        prompt.routes.useAddress(for: "fc-cosmo")
        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: Self.unreached))
        try await Task.sleep(for: .milliseconds(100))
        #expect(asks == 0)
    }

    /// Runners that fail together, as after a wake, are one question naming
    /// each (M3), and Connect Directly moves them all.
    @Test func runnersThatFailTogetherAreOneQuestion() async throws {
        let config = try await joined(["cosmo": "192.168.1.180", "mini": "10.0.0.4"])
        let prompt = prompt(config)
        prompt.gathering = 0.2
        var asked: [RouteQuestion] = []
        var answer: ((Bool) -> Void)?
        prompt.ask = { question, reply in asked.append(question); answer = reply }
        var redialed: [String] = []
        prompt.reconnect = { redialed.append($0) }

        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: Self.unreached))
        prompt.heard(target: "fc-mini", state: .unreachable(reason: Self.unreached))
        await eventually { !asked.isEmpty }
        #expect(asked.count == 1)
        #expect(Set(asked[0].runners.map(\.target)) == ["fc-cosmo", "fc-mini"])
        #expect(asked[0].message.contains("fc-cosmo at 192.168.1.180"), "\(asked[0].message)")

        answer?(true)
        #expect(Set(redialed) == ["fc-cosmo", "fc-mini"])
    }

    /// One question on screen at a time: a runner that fails while one is up
    /// waits for its answer.
    @Test func oneQuestionAtATime() async throws {
        let config = try await joined(["cosmo": "192.168.1.180", "mini": "10.0.0.4"])
        let prompt = prompt(config)
        var asked: [RouteQuestion] = []
        var answer: ((Bool) -> Void)?
        prompt.ask = { question, reply in asked.append(question); answer = reply }

        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: Self.unreached))
        await eventually { asked.count == 1 }
        prompt.heard(target: "fc-mini", state: .unreachable(reason: Self.unreached))
        try await Task.sleep(for: .milliseconds(100))
        #expect(asked.count == 1, "a second question while one was up")

        answer?(false)
        await eventually { asked.count == 2 }
        #expect(asked.last?.runners.map(\.target) == ["fc-mini"])
    }

    /// Return answers Not Now: a keystroke meant for a terminal pane never
    /// moves a runner (H2).
    @Test func returnAnswersNotNow() {
        let alert = RoutePrompt.makeAlert(RouteQuestion(runners: [.init(target: "fc-cosmo", address: "10.0.0.4")]))
        #expect(alert.buttons.map(\.title) == ["Not Now", "Connect Directly"])
        #expect(alert.buttons[0].keyEquivalent == "\r")
        #expect(alert.buttons[1].keyEquivalent == "")
    }

    /// The store's update notes a failed Tailcat runner for the prompt (M4):
    /// a client whose read failed with the pipe's marker is asked about.
    @Test func theFleetStoreHandsAFailedRunnerToThePrompt() async throws {
        let config = try await joined()
        let prompt = prompt(config)
        var asked: [RouteQuestion] = []
        prompt.ask = { question, _ in asked.append(question) }
        let client = DaemonClient(target: "fc-cosmo", notifications: NotificationCenter())
        client.commandRunnerForTesting = { _ in (nil, Self.unreached) }
        await client.refresh()
        #expect(client.state == .unreachable(reason: Self.unreached))

        let store = FleetStore(clients: ["fc-cosmo": client], routePrompt: prompt)
        await eventually { !asked.isEmpty }
        #expect(asked.first?.runners.map(\.target) == ["fc-cosmo"])
        _ = store
    }

    // MARK: - Fixtures

    private func granted(_ label: String, host: String?) -> CeremonyRunner {
        var runner = CeremonyRunner(
            id: "you@\(label)", label: label, alias: "", user: "you", host_key: JoinApplyTests.pin(1),
            reach: .tailcat(token: "tc-secret"), pending: false)
        runner.alternative = host.map { .direct(host: $0, port: 22) }
        return runner
    }

    private func manifest(_ runners: [CeremonyRunner]) -> CeremonyManifest {
        CeremonyManifest(v: 2, ceremony: "c", account: "a", channel: "release", target: "t", runners: runners)
    }

    private func joined(host: String? = "192.168.1.180") async throws -> URL {
        let config = try scratch()
        _ = await join(config).run(
            manifest([granted("cosmo", host: host)]), configIdentity: JoinApplyTests.identity
        ) { $0.alias }
        return config
    }

    /// Several runners, each with its own pin and address.
    private func joined(_ runners: [String: String]) async throws -> URL {
        let config = try scratch()
        let list = runners.sorted(by: { $0.key < $1.key }).enumerated().map { index, entry in
            var runner = self.granted(entry.key, host: entry.value)
            runner.host_key = JoinApplyTests.pin(index + 1)
            return runner
        }
        _ = await join(config).run(manifest(list), configIdentity: JoinApplyTests.identity) { $0.alias }
        return config
    }

    private func join(_ config: URL) -> JoinApply {
        var join = JoinApply(keeper: JoinApplyTests.Keeper())
        join.alternatives = alternatives
        join.program = { JoinApplyTests.program }
        join.patternsInUse = { [] }
        join.readEntries = { SshConfig.entries(in: config) }
        join.writeConfig = { try SshConfig.write($0, identity: $1, into: config) }
        return join
    }

    private func switcher(_ config: URL) -> RouteSwitch {
        var routes = RouteSwitch()
        routes.alternatives = alternatives
        routes.readEntries = { SshConfig.entries(in: config) }
        routes.writeConfig = { try SshConfig.write($0, identity: $1, into: config) }
        return routes
    }

    private func scratch() throws -> URL {
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("farcooler-route-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory.appendingPathComponent("config")
    }

    private func written(_ config: URL) -> String {
        (try? String(contentsOf: config, encoding: .utf8)) ?? ""
    }
}
