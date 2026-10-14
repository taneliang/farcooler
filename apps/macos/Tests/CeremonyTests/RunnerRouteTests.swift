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

    /// Tailcat fails: the owner's question is asked, once, and nothing moves
    /// until it is answered.
    @Test func aFailedTunnelAsksOnceAndMovesNothingByItself() async throws {
        let config = try await joined()
        let prompt = RoutePrompt()
        prompt.routes = switcher(config)
        var asked: [DirectRouteOffer] = []
        prompt.ask = { offer, _ in asked.append(offer) }
        prompt.reconnect = { _ in Issue.record("dialed without being asked") }

        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: "Connection closed by UNKNOWN port 65535"))
        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: "Connection closed by UNKNOWN port 65535"))

        #expect(asked == [DirectRouteOffer(address: "192.168.1.180")])
        #expect(asked.first?.sentence
            == "Connecting through Tailcat failed. Connect directly to 192.168.1.180 instead?")
        #expect(written(config).contains("  ProxyCommand "), "moved to the address on its own")
    }

    /// Connect Directly moves the runner and dials it again on its address.
    @Test func connectDirectlyMovesTheRunnerAndRedials() async throws {
        let config = try await joined()
        let prompt = RoutePrompt()
        prompt.routes = switcher(config)
        prompt.ask = { _, answer in answer(true) }
        var redialed: [String] = []
        prompt.reconnect = { redialed.append($0) }

        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: "no answer"))

        #expect(redialed == ["fc-cosmo"])
        #expect(written(config).contains("HostName 192.168.1.180"))
    }

    /// Not Now leaves it on Tailcat, and it is asked again only after the
    /// runner has answered once.
    @Test func notNowLeavesItOnTailcatUntilItAnswersAgain() async throws {
        let config = try await joined()
        let prompt = RoutePrompt()
        prompt.routes = switcher(config)
        var asks = 0
        prompt.ask = { _, answer in asks += 1; answer(false) }
        prompt.reconnect = { _ in Issue.record("dialed on a Not Now") }

        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: "no answer"))
        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: "no answer"))
        prompt.heard(target: "fc-cosmo", state: .connected)
        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: "no answer"))

        #expect(asks == 2)
        #expect(written(config).contains("  ProxyCommand "))
    }

    /// Never asked about a runner already on its address, about one that
    /// answered and refused, or about this Mac.
    @Test func nothingIsAskedWhereTheAddressCannotHelp() async throws {
        let config = try await joined()
        let prompt = RoutePrompt()
        prompt.routes = switcher(config)
        prompt.ask = { _, _ in Issue.record("asked where the address cannot help") }

        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: "you@cosmo: Permission denied (publickey)."))
        prompt.heard(target: "", state: .unreachable(reason: "no answer"))
        prompt.heard(target: "fc-cosmo", state: .notInstalled)
        prompt.routes.useAddress(for: "fc-cosmo")
        prompt.heard(target: "fc-cosmo", state: .unreachable(reason: "no answer"))
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
