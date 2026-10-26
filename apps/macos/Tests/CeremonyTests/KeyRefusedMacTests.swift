import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A runner that let this Mac in and now refuses its key (ov-818): after the
/// refusals hold, and a verbose dial proves the refusal is the runner's own
/// (review S1), the window says this Mac no longer has access, drops the
/// runner's work and checks again slowly. A refusal that passes, or one the
/// probe can't prove, is still a runner that can't be reached, retried fast,
/// with its work on screen.
///
/// Driven through `refresh()`, the call that reads ssh's refusal, against a
/// stubbed CLI that answers with OpenSSH's own words, and a probe that answers
/// with real `ssh -v` output from `test/fixtures/ssh-refusals`.
@MainActor
struct KeyRefusedMacTests {
    /// What `farcooler --host` passes through when sshd refuses the key.
    static let refused = "you@cosmo: Permission denied (publickey).\nCould not reach you@cosmo over ssh."
    static let fleet = Data(
        #"{"runtime_healthy":true,"live_panes":0,"worktrees":[{"id":"w-1","short":"w1","task":"fix-it","branch":"fix-it","worktree":"/tmp/fix-it","state":"active","terminals":[]}]}"#
            .utf8)

    @MainActor
    final class Runner {
        var refusing = false
        func answer(_ args: [String]) -> (data: Data?, message: String?) {
            if refusing { return (nil, KeyRefusedMacTests.refused) }
            if args.filter({ $0 != "--json" }) == ["worktree", "list"] { return (KeyRefusedMacTests.fleet, nil) }
            return (Data(), nil)
        }
    }

    /// One `ssh -v` sample, by name.
    static func sample(_ name: String) throws -> String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try String(
            contentsOf: root.appendingPathComponent("test/fixtures/ssh-refusals/\(name).txt"), encoding: .utf8)
    }

    /// A client on `you@cosmo` with its own memory, clock and probe, retries
    /// off so nothing dials behind the test's back. `probes` counts the probes.
    private func client(
        _ runner: Runner, _ defaults: UserDefaults, probe: String, probes: Counter = Counter(),
        clock: Clock
    ) -> DaemonClient {
        let client = DaemonClient(target: "you@cosmo", notifications: NotificationCenter())
        client.admissions = defaults
        client.now = { clock.at }
        client.commandRunnerForTesting = { args in runner.answer(args) }
        client.refusalProbe = { _ in
            probes.count += 1
            return probe
        }
        client.stopEvents()
        return client
    }

    @MainActor final class Counter { var count = 0 }

    /// The test's clock, a reference so the client reads it while a test moves it.
    @MainActor final class Clock { var at = Date(timeIntervalSince1970: 1_800_000_000) }

    private func defaults() throws -> UserDefaults {
        try #require(ScratchDefaults.suite("KeyRefusedMacTests.\(UUID().uuidString)"))
    }

    /// The backoff's spacing, in seconds from the first refusal.
    private static let spacing: [TimeInterval] = [0, 2, 6, 14]

    /// Refused from `start` on, at each of `offsets`.
    private func refuse(_ client: DaemonClient, _ runner: Runner, from start: Date, at offsets: [TimeInterval], clock: Clock) async {
        runner.refusing = true
        for offset in offsets {
            clock.at = start.addingTimeInterval(offset)
            await client.refresh()
        }
    }

    @Test func aKeyHeldRefusedAndProvenIsNoAccessWithNothingOfTheTeamLeft() async throws {
        let defaults = try defaults()
        let runner = Runner()
        let clock = Clock()
        let probes = Counter()
        let client = client(runner, defaults, probe: try Self.sample("revoked"), probes: probes, clock: clock)

        await client.refresh()
        #expect(client.state == .connected)
        #expect(!client.fleet.worktrees.isEmpty)
        client.repositories = [Repository(id: "r", short: "r1", displayName: "overnight", remote: "", repositoryRootId: "root")]
        client.layouts = ["w-1": []]
        client.planNews = ["ws": 1]
        let board = TaskBoardStore(client: client, workspace: Self.billing)
        #expect(board.isHeld(by: ["you@cosmo": client]))

        let start = clock.at
        await refuse(client, runner, from: start, at: Self.spacing, clock: clock)

        #expect(client.state == .noAccess, "\(client.state)")
        #expect(probes.count == 1, "probed \(probes.count) times")
        #expect(client.fleet.worktrees.isEmpty, "the runner's work stayed on screen")
        #expect(client.repositories.isEmpty && client.layouts.isEmpty && client.planNews.isEmpty)
        #expect(!board.isHeld(by: ["you@cosmo": client]), "an open board of the runner's stayed held")
        #expect(ContentView.shutOut(client.state), "the window would draw the runner's workspace")
        #expect(DaemonClient.waitsLong(client.state), "a revoked key is retried fast")
        #expect(client.state.refusal == RunnerPeople.noAccessBody)
    }

    /// Every way to "Permission denied (publickey)" that isn't the runner
    /// refusing this Mac's key, read through the probe: never no access, the
    /// work kept, retried fast.
    @Test(arguments: [
        "agent-refused", "agent-unanswered", "passphrase-no-agent", "bastion-refused", "impostor",
        "host-key-changed", "penalized",
    ])
    func aRefusalTheProbeCannotProveIsNotNoAccess(_ name: String) async throws {
        let defaults = try defaults()
        let runner = Runner()
        let clock = Clock()
        let probes = Counter()
        let client = client(runner, defaults, probe: try Self.sample(name), probes: probes, clock: clock)
        await client.refresh()

        await refuse(client, runner, from: clock.at, at: Self.spacing + [44, 74, 104], clock: clock)

        #expect(client.state == .unreachable(reason: Self.refused), "\(name): \(client.state)")
        #expect(probes.count >= 2, "\(name): the probe wasn't asked again after \(probes.count)")
        #expect(!client.fleet.worktrees.isEmpty, "\(name): the work was dropped")
        #expect(!client.teamWorkForgotten)
        #expect(!DaemonClient.waitsLong(client.state))
    }

    /// Settled once, it stays settled through the refusals after, without
    /// probing each one.
    @Test func noAccessHoldsThroughTheNextRefusals() async throws {
        let defaults = try defaults()
        let runner = Runner()
        let clock = Clock()
        let probes = Counter()
        let client = client(runner, defaults, probe: try Self.sample("revoked"), probes: probes, clock: clock)
        await client.refresh()
        let start = clock.at
        await refuse(client, runner, from: start, at: Self.spacing, clock: clock)
        #expect(client.state == .noAccess)

        await refuse(client, runner, from: start, at: [15, 16, 17], clock: clock)
        #expect(client.state == .noAccess, "\(client.state)")
        #expect(probes.count == 1)
    }

    /// Added back: Check Again (or the slow check) reads the runner, and
    /// everything comes back.
    @Test func aRunnerThatLetsThisMacInAgainIsReadAgain() async throws {
        let defaults = try defaults()
        let runner = Runner()
        let clock = Clock()
        let client = client(runner, defaults, probe: try Self.sample("revoked"), clock: clock)
        await client.refresh()
        await refuse(client, runner, from: clock.at, at: Self.spacing, clock: clock)
        #expect(client.state == .noAccess)

        runner.refusing = false
        await client.refresh()
        #expect(client.state == .connected)
        #expect(!client.fleet.worktrees.isEmpty, "the fleet wasn't read again")
        #expect(!client.teamWorkForgotten)
        #expect(client.keyRefusals.refusals == 0)
    }

    /// Being let in is remembered: a Mac removed while the app was closed
    /// says so on the next launch's reads.
    @Test func aRelaunchRemembersTheRunnerLetThisMacIn() async throws {
        let defaults = try defaults()
        let runner = Runner()
        let clock = Clock()
        await client(runner, defaults, probe: try Self.sample("revoked"), clock: clock).refresh()

        let relaunched = client(runner, defaults, probe: try Self.sample("revoked"), clock: clock)
        await refuse(relaunched, runner, from: clock.at, at: Self.spacing, clock: clock)
        #expect(relaunched.state == .noAccess, "\(relaunched.state)")
    }

    /// The transient case: refused a moment, then let in. Never no access,
    /// retried on the backoff throughout, and the work stays until it's read
    /// again.
    @Test func aRefusalThatPassesIsStillUnreachableAndRetriedFast() async throws {
        let defaults = try defaults()
        let runner = Runner()
        let clock = Clock()
        let client = client(runner, defaults, probe: try Self.sample("revoked"), clock: clock)
        await client.refresh()

        for round in 0..<3 {
            runner.refusing = true
            let start = clock.at
            for offset in Self.spacing.prefix(3) {
                clock.at = start.addingTimeInterval(offset)
                await client.refresh()
                #expect(client.state == .unreachable(reason: Self.refused), "round \(round): \(client.state)")
                #expect(!DaemonClient.waitsLong(client.state))
                #expect(!client.fleet.worktrees.isEmpty)
            }
            runner.refusing = false
            clock.at = clock.at.addingTimeInterval(8)
            await client.refresh()
            #expect(client.state == .connected)
        }
    }

    /// A runner this Mac never got into is a stranger's refusal: as before,
    /// unreachable, nothing said about access it never had, and never probed.
    @Test func aRunnerThatNeverLetThisMacInIsNotNoAccess() async throws {
        let defaults = try defaults()
        let runner = Runner()
        runner.refusing = true
        let clock = Clock()
        let probes = Counter()
        let client = client(runner, defaults, probe: try Self.sample("revoked"), probes: probes, clock: clock)
        await refuse(client, runner, from: clock.at, at: Self.spacing + [60, 600], clock: clock)
        #expect(client.state == .unreachable(reason: Self.refused))
        #expect(probes.count == 0)
    }

    /// The probe asks nothing of the person, reuses no master, and checks the
    /// host key strictly whatever the config says.
    @Test func theProbeIsStrictAndQuiet() {
        let args = DaemonClient.probeArguments("you@cosmo")
        for option in ["BatchMode=yes", "ControlMaster=no", "ControlPath=none", "StrictHostKeyChecking=yes"] {
            #expect(args.contains(option), "\(option): \(args)")
        }
        #expect(args.contains("-v"))
        #expect(args.suffix(3) == ["--", "you@cosmo", "true"])
    }

    @Test func theScreensSayNoAccessAndNameTheRunner() {
        #expect(FleetPlaceholder.noAccessTitle("studio") == "You No Longer Have Access to studio")
        #expect(FleetPlaceholder.noAccessTitle("") == RunnerPeople.noAccessTitle)
        let phase = FleetPlaceholder.phase(
            hasWorktrees: false, localLoaded: true, localError: nil, hasRepositories: true,
            restoringOn: "studio", trouble: RunnerPeople.noAccessBody, waited: 20, noAccessTo: "studio")
        #expect(phase == .noAccess("studio"))
        #expect(RunnerStatusItem.problem(.noAccess) == .noAccess)
        #expect(RunnerStatusItem.label(troubles: [.init(host: "studio", problem: .noAccess)], stale: []) == "No access to studio")
        #expect(ContentView.shutOut(.removed) && ContentView.shutOut(.noAccess))
        #expect(!ContentView.shutOut(.connected) && !ContentView.shutOut(.unreachable(reason: "")) && !ContentView.shutOut(nil))
    }

    private static let billing = WorkspaceSummary(
        id: "0198f2c0-0000-7000-8000-0000000000dd", name: "Billing", taskPrefix: "bil", isMain: false,
        ordinal: 1, repository: "r", orchestrator: nil)
}
