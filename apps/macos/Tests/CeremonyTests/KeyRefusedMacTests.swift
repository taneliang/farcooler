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

    /// What the CLI shows when the agent wouldn't sign: the agent's trouble,
    /// then the same refusal.
    static let agentWouldNotSign = """
        sign_and_send_pubkey: signing failed for ED25519 "/Users/e/.ssh/farcooler-mac" from agent: agent refused operation
        you@cosmo: Permission denied (publickey).
        Could not reach you@cosmo over ssh.
        """

    /// The key the runner named, the one the samples' runner refused.
    static let key = "SHA256:GXr0aYtmcLgDEc2veRwu0VEX475UEN/n3nFeTP//P+o"

    @MainActor
    final class Runner {
        var refusing = false
        /// What a refused read's stderr says.
        var refusal = KeyRefusedMacTests.refused
        /// The runner released this Mac but its ssh master still serves it
        /// (ov-784, `a_real_sshd_master_outlives_a_revoke`).
        var released = false
        func answer(_ args: [String]) -> (data: Data?, message: String?) {
            if refusing { return (nil, refusal) }
            if released {
                if args.filter({ $0 != "--json" }) == ["member", "list"] {
                    return (Data(#"{"members":[],"claude_account":"","you":"(released)"}"#.utf8), nil)
                }
                return (nil, "error: someone else's to do on this runner\ncode: not-yours")
            }
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
        named: String? = KeyRefusedMacTests.key, clock: Clock
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
        // What `status --json` says once the link is up (`readDaemonBuild`).
        client.learnKey(named)
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
        "stray-key-agent-missing", "stray-key-agent-empty", "agent-refused", "agent-unanswered",
        "passphrase-no-agent", "proxyjump", "impostor", "host-key-changed", "penalized",
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

    /// The stray key (review 1): a missing agent, a stray `~/.ssh/id_*`
    /// offered and refused under the right host key. Never counted.
    @Test(arguments: ["stray-key-agent-missing", "stray-key-agent-empty"])
    func aStrayKeyRefusedIsNeverNoAccess(_ name: String) async throws {
        let defaults = try defaults()
        let runner = Runner()
        let clock = Clock()
        let client = client(runner, defaults, probe: try Self.sample(name), clock: clock)
        await client.refresh()
        await refuse(client, runner, from: clock.at, at: Self.spacing + [44, 74, 104, 134], clock: clock)
        #expect(client.state == .unreachable(reason: Self.refused), "\(name): \(client.state)")
        #expect(!client.fleet.worktrees.isEmpty)
    }

    /// One probe in a stretch of refusals under 30 s apart (review 5): the
    /// limit at the call site, which every other test here would pass without.
    @Test func refusalsUnderThirtySecondsApartProbeOnce() async throws {
        let defaults = try defaults()
        let runner = Runner()
        let clock = Clock()
        let probes = Counter()
        let client = client(runner, defaults, probe: try Self.sample("agent-refused"), probes: probes, clock: clock)
        await client.refresh()
        await refuse(client, runner, from: clock.at, at: Self.spacing + [15, 16, 20, 43], clock: clock)
        #expect(probes.count == 1, "probed \(probes.count) times in 29 s")
    }

    /// A runner that never named the key this Mac came in with (one from
    /// before) can't be proven, so it's never probed.
    @Test func noKeyNamedIsNeverProbed() async throws {
        let defaults = try defaults()
        let runner = Runner()
        let clock = Clock()
        let probes = Counter()
        let client = client(runner, defaults, probe: try Self.sample("revoked"), probes: probes, named: nil, clock: clock)
        await client.refresh()
        await refuse(client, runner, from: clock.at, at: Self.spacing + [44], clock: clock)
        #expect(probes.count == 0)
        #expect(client.state == .unreachable(reason: Self.refused))
    }

    /// The CLI already said the agent wouldn't sign (review 3): no probe, so
    /// no agent is asked to put up its prompt again.
    @Test func anAgentInTroubleIsNotProbed() async throws {
        let defaults = try defaults()
        let runner = Runner()
        runner.refusal = Self.agentWouldNotSign
        let clock = Clock()
        let probes = Counter()
        let client = client(runner, defaults, probe: try Self.sample("revoked"), probes: probes, clock: clock)
        await client.refresh()
        await refuse(client, runner, from: clock.at, at: Self.spacing + [44, 74], clock: clock)
        #expect(probes.count == 0)
        #expect(client.state == .unreachable(reason: Self.agentWouldNotSign))
    }

    /// The S6 case, end to end (review 4): a Mac connected when only this
    /// device was taken off its member's list. Over its old ssh master it
    /// reads `(released)` and says it was removed; once the master ends, the
    /// refusals hold and the probe proves them, and it moves to no access,
    /// whose words don't say the member was removed.
    @Test func removedOverTheMasterThenProvenIsNoAccess() async throws {
        let defaults = try defaults()
        let runner = Runner()
        let clock = Clock()
        let client = client(runner, defaults, probe: try Self.sample("revoked"), clock: clock)
        await client.refresh()
        #expect(client.state == .connected)

        runner.released = true
        await client.refresh()
        #expect(client.state == .removed, "\(client.state)")
        #expect(client.fleet.worktrees.isEmpty)

        runner.released = false
        let start = clock.at
        await refuse(client, runner, from: start, at: Array(Self.spacing.prefix(3)), clock: clock)
        #expect(client.state == .removed, "short of proof it said something else: \(client.state)")
        await refuse(client, runner, from: start, at: [14], clock: clock)
        #expect(client.state == .noAccess, "\(client.state)")
        #expect(client.state.refusal == RunnerPeople.noAccessBody)
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

    /// The probe runs the `ssh` first on the PATH it's given (the login
    /// shell's, as the CLI's is), and is killed by its PID at the deadline,
    /// even with a child of its holding the pipe open (review 2 and 3).
    @Test func theProbeRunsTheCLIsSshAndStopsAtItsDeadline() async throws {
        let bin = FileManager.default.temporaryDirectory.appendingPathComponent("fc818-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: bin) }
        let ssh = bin.appendingPathComponent("ssh")
        // Says which ssh it is, leaves a child holding stderr, then hangs.
        try "#!/bin/sh\necho \"the CLI's ssh: $*\" >&2\n(sleep 6 >&2 &)\nexec sleep 6\n".write(
            to: ssh, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: ssh.path)

        let started = Date()
        let said = await DaemonClient.probeRefusal(
            "you@cosmo", environment: ["PATH": bin.path + ":/usr/bin:/bin"], deadline: 1)
        let took = Date().timeIntervalSince(started)
        #expect(said.contains("the CLI's ssh: -v -n"), "\(said)")
        #expect(said.contains("you@cosmo true"))
        #expect(took < 4, "the probe outlived its deadline, or waited on its child: \(took) s")
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
