import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A runner that let this Mac in and now refuses its key (ov-818): after the
/// refusals hold, the window says this Mac no longer has access, drops the
/// runner's work and checks again slowly. A refusal that passes is still a
/// runner that can't be reached, retried fast, with its work on screen.
///
/// Driven through `refresh()`, the call that reads ssh's refusal, against a
/// stubbed CLI that answers with OpenSSH's own words.
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

    /// A client on `you@cosmo` with its own memory and clock, retries off so
    /// nothing dials behind the test's back.
    private func client(_ runner: Runner, _ defaults: UserDefaults, clock: @escaping () -> Date) -> DaemonClient {
        let client = DaemonClient(target: "you@cosmo", notifications: NotificationCenter())
        client.admissions = defaults
        client.now = clock
        client.commandRunnerForTesting = { args in runner.answer(args) }
        client.stopEvents()
        return client
    }

    private func defaults() throws -> (UserDefaults, String) {
        let suite = "KeyRefusedMacTests.\(UUID().uuidString)"
        return (try #require(UserDefaults(suiteName: suite)), suite)
    }

    /// The backoff's spacing, in seconds from the first refusal.
    private static let spacing: [TimeInterval] = [0, 2, 6, 14]

    @Test func aKeyHeldRefusedByARunnerThatLetThisMacInIsNoAccess() async throws {
        let (defaults, suite) = try defaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let runner = Runner()
        var at = Date(timeIntervalSince1970: 1_800_000_000)
        let client = client(runner, defaults) { at }

        await client.refresh()
        #expect(client.state == .connected)
        #expect(!client.fleet.worktrees.isEmpty)

        runner.refusing = true
        let start = at
        for offset in Self.spacing {
            at = start.addingTimeInterval(offset)
            await client.refresh()
        }

        #expect(client.state == .noAccess, "\(client.state)")
        #expect(client.fleet.worktrees.isEmpty, "the runner's work stayed on screen")
        #expect(DaemonClient.waitsLong(client.state), "a revoked key is retried fast")
        #expect(client.state.refusal == RunnerPeople.noAccessBody)
    }

    /// Being let in is remembered: a Mac removed while the app was closed
    /// says so on the next launch's reads.
    @Test func aRelaunchRemembersTheRunnerLetThisMacIn() async throws {
        let (defaults, suite) = try defaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let runner = Runner()
        var at = Date(timeIntervalSince1970: 1_800_000_000)
        await client(runner, defaults) { at }.refresh()

        runner.refusing = true
        let relaunched = client(runner, defaults) { at }
        let start = at
        for offset in Self.spacing {
            at = start.addingTimeInterval(offset)
            await relaunched.refresh()
        }
        #expect(relaunched.state == .noAccess, "\(relaunched.state)")
    }

    /// The transient case: refused a moment, then let in. Never no access,
    /// retried on the backoff throughout, and the work stays until it's read
    /// again.
    @Test func aRefusalThatPassesIsStillUnreachableAndRetriedFast() async throws {
        let (defaults, suite) = try defaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let runner = Runner()
        var at = Date(timeIntervalSince1970: 1_800_000_000)
        let client = client(runner, defaults) { at }
        await client.refresh()

        for round in 0..<3 {
            runner.refusing = true
            let start = at
            for offset in Self.spacing.prefix(3) {
                at = start.addingTimeInterval(offset)
                await client.refresh()
                #expect(client.state == .unreachable(reason: Self.refused), "round \(round): \(client.state)")
                #expect(!DaemonClient.waitsLong(client.state))
                #expect(!client.fleet.worktrees.isEmpty)
            }
            runner.refusing = false
            at = at.addingTimeInterval(8)
            await client.refresh()
            #expect(client.state == .connected)
        }
    }

    /// A runner this Mac never got into is a stranger's refusal: as before,
    /// unreachable, and nothing said about access it never had.
    @Test func aRunnerThatNeverLetThisMacInIsNotNoAccess() async throws {
        let (defaults, suite) = try defaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let runner = Runner()
        runner.refusing = true
        var at = Date(timeIntervalSince1970: 1_800_000_000)
        let client = client(runner, defaults) { at }
        let start = at
        for offset in Self.spacing + [60, 600] {
            at = start.addingTimeInterval(offset)
            await client.refresh()
        }
        #expect(client.state == .unreachable(reason: Self.refused))
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
    }
}
