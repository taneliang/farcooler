import Foundation
import Testing

@testable import Far_Cooler

/// Each action's result is its own (ov-135).
///
/// The banner used to read one client's `lastError` after every action.
/// Every command wrote it and every good `worktree list` cleared it, so:
/// - a Stop the runner refused vanished, because the refresh after it worked;
/// - a later action that worked showed whatever an earlier command had failed
///   with;
/// - what it showed was the CLI's stderr.
///
/// Each test here drives the client through `ActionOutcomes.perform`, the
/// call `ContentView.act` makes, against a stubbed CLI.
@MainActor
struct ActionOutcomeTests {
    /// A CLI where `fails` is refused with its stderr, `worktree list` reads
    /// an empty fleet, and everything else works and says nothing.
    @MainActor
    final class Runner {
        var calls: [[String]] = []
        var fails: [String: String] = [:]

        func answer(_ args: [String]) -> (data: Data?, message: String?) {
            calls.append(args)
            let words = args.filter { $0 != "--json" }
            for (prefix, stderr) in fails where words.joined(separator: " ").hasPrefix(prefix) {
                return (nil, stderr)
            }
            if words == ["worktree", "list"] {
                return (Data(#"{"runtime_healthy":true,"live_panes":0,"worktrees":[]}"#.utf8), nil)
            }
            if words.first == "layout" {
                return (Data(#"{"worktree":"w-1","groups":[]}"#.utf8), nil)
            }
            return (Data(), nil)
        }
    }

    private func client(_ runner: Runner) -> DaemonClient {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in runner.answer(args) }
        return client
    }

    private static func key(_ verb: ActionVerb, _ target: String) -> ActionKey {
        ActionKey(verb: verb, host: "", target: target)
    }

    /// The bug as it was reported: Stop refused, the refresh after it fine,
    /// and nothing on screen.
    @Test func aRefusedStopSurvivesTheRefreshAfterIt() async {
        let runner = Runner()
        runner.fails["terminal stop"] = "error: permission denied for control\ncode: scope-denied"
        let client = client(runner)
        let outcomes = ActionOutcomes()

        await outcomes.perform(Self.key(.stop, "t-1"), subject: "“agent”", on: client) {
            await $0.stop(terminal: "t1")
        }

        #expect(runner.calls.contains { $0.starts(with: ["worktree", "list"]) }, "the refresh ran: \(runner.calls)")
        #expect(client.fleetError == nil, "and it read the fleet")
        #expect(
            outcomes.shown == [
                ActionFailure(
                    key: Self.key(.stop, "t-1"),
                    sentence: "Couldn’t stop “agent”. This runner lets Far Cooler see it but not change it.")
            ])
    }

    /// It stays until the same action on the same target says otherwise, or
    /// it's dismissed. A different action, or the same one on another
    /// target, leaves it.
    @Test func aResultStaysUntilItsOwnActionSupersedesIt() async {
        let runner = Runner()
        runner.fails["terminal restart t1"] = "error: no such pane\ncode: not-found"
        let client = client(runner)
        let outcomes = ActionOutcomes()

        await outcomes.perform(Self.key(.restart, "t-1"), subject: "“one”", on: client) {
            await $0.restart(terminal: "t1")
        }
        await outcomes.perform(Self.key(.restart, "t-2"), subject: "“two”", on: client) {
            await $0.restart(terminal: "t2")
        }
        await outcomes.perform(Self.key(.hide, "w-1"), subject: "“fix it”", on: client) {
            await $0.hideWorktree("w1")
        }
        #expect(outcomes.shown.map(\.key) == [Self.key(.restart, "t-1")])

        runner.fails = [:]
        await outcomes.perform(Self.key(.restart, "t-1"), subject: "“one”", on: client) {
            await $0.restart(terminal: "t1")
        }
        #expect(outcomes.shown.isEmpty, "restarting it again, and working, takes it down")

        outcomes.settle(Self.key(.stop, "t-1"), failure: "Couldn’t stop “one”.")
        outcomes.dismiss(Self.key(.stop, "t-1"))
        #expect(outcomes.shown.isEmpty)
    }

    /// The other half of the bug: a failure from before the action, here a
    /// layout read made outside any action, did not happen to the action,
    /// and doesn't show on it. The action is a layout command, which reads
    /// no fleet after it, so nothing in between clears an old failure.
    @Test func aStaleFailureDoesNotShowOnALaterAction() async {
        let runner = Runner()
        runner.fails["layout show"] = "error: no such worktree\ncode: not-found"
        let client = client(runner)
        let outcomes = ActionOutcomes()
        let worktree = Worktree(
            id: "w-1", short: "w1", task: "fix-it", branch: "fix-it", repository: "repo",
            host: "", path: "/tmp/fix-it", state: "active", terminals: [])

        await client.refreshLayout(worktree)
        await outcomes.perform(Self.key(.arrange, "w-1"), subject: "“fix it”", on: client) {
            await $0.selectLayout("@2", in: worktree)
        }

        #expect(outcomes.shown.isEmpty, "\(outcomes.shown)")
    }

    /// What the banner says is the app's sentence, chosen by the `code:`
    /// word; the runner's own words never reach it.
    @Test func noRawStderrIsShown() async {
        let runner = Runner()
        let stderr = "error: tmux server exited unexpectedly (status 1)\ncode: tmux-unavailable"
        runner.fails["terminal stop"] = stderr
        let outcomes = ActionOutcomes()

        await outcomes.perform(Self.key(.stop, "t-1"), subject: "“agent”", on: client(runner)) {
            await $0.stop(terminal: "t1")
        }

        let shown = outcomes.shown.map(\.sentence)
        #expect(shown == ["Couldn’t stop “agent”. The runner can’t reach tmux. Install tmux there, then try again."])
        #expect(!shown.contains { $0.contains("error:") || $0.contains("tmux server exited") || $0.contains("code:") })
    }

    /// The mapping, pinned: each word the daemon sends, and stderr with no
    /// word at all, which is ssh's or the CLI's and goes to the log.
    @Test func eachCodeHasItsSentence() {
        let cases: [(String, String)] = [
            ("code: not-found", "It isn’t on this runner anymore."),
            ("code: running-processes", "Something is still running in it. Stop it first, then try again."),
            ("code: tmux-unavailable", "The runner can’t reach tmux. Install tmux there, then try again."),
            ("code: capability-unsupported", "This runner’s Far Cooler is too old for this. Update it there, then try again."),
            ("code: version-incompatible", "This runner’s Far Cooler is too old for this. Update it there, then try again."),
            ("code: scope-denied", "This runner lets Far Cooler see it but not change it."),
            ("code: auth-required", "This runner didn’t accept Far Cooler’s sign-in."),
            ("code: resource-conflict\nwhat: not_held", "It changed while you were doing that. Try again."),
            ("code: host-offline", "The runner is offline."),
            ("code: agent-not-connected", "Its agent isn’t connected right now."),
            ("code: operation-failed", "The runner tried and it didn’t work. Try again."),
            (
                "code: invalid-argument",
                "The runner couldn’t take the request as Far Cooler sent it. That’s a problem in the app, not in anything you did."
            ),
            ("ssh: connect to host runner port 22: Connection refused", "Check that the runner is reachable, then try again."),
            ("error: unexpected argument '--frobnicate' found", "Check that the runner is reachable, then try again."),
        ]
        for (stderr, reason) in cases {
            #expect(ActionCopy.reason("error: whatever the daemon said\n" + stderr) == reason, "\(stderr)")
        }
        #expect(
            ActionCopy.sentence(.close, subject: "“agent”", message: "ssh: Could not resolve hostname runner")
                == "Couldn’t close “agent”. Check that the runner is reachable, then try again.")
    }

    /// A runner already known not to answer refuses the click before it's
    /// sent, and says so in a sentence, not in the stderr that made it
    /// unreachable.
    @Test func aRefusalBeforeSendingIsASentence() {
        let refused = ActionCopy.refused(.unreachable(reason: "ssh: connect to host runner port 22: Operation timed out"))
        #expect(refused == "That runner can’t be reached right now. Far Cooler will keep trying.")
        #expect(ActionCopy.refused(.notInstalled) == "Far Cooler isn’t installed on that runner yet.")
        #expect(ActionCopy.refused(nil) == "That runner isn’t set up anymore.")
    }
}
