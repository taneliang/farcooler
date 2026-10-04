import AgentKit
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
                    sentence: "Couldn’t stop “agent”. \(RunnerRefusal.scopeDenied.sentence)")
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
        #expect(shown == ["Couldn’t stop “agent”. \(RunnerRefusal.tmuxUnavailable.sentence)"])
        #expect(!shown.contains { $0.contains("error:") || $0.contains("tmux server exited") || $0.contains("code:") })
    }

    /// The mapping, pinned: every word `farcooler_core::error::word` has,
    /// a word this build doesn't know, and stderr with no word, which is
    /// ssh's, the transport's or the CLI's and goes to the log.
    @Test func eachCodeHasItsSentence() {
        let app = "The runner couldn’t take the request as Far Cooler sent it. That’s a problem in the app, not in anything you did."
        let reach = "Check that the runner is reachable, then try again."
        let neutral = "Something went wrong. Try again."
        let cases: [(String, String)] = [
            ("code: not-found", RunnerRefusal.notFound.sentence),
            ("code: running-processes", RunnerRefusal.runningProcesses.sentence),
            ("code: dirty-worktree", "It has changes that aren’t committed. Commit or discard them, then try again."),
            ("code: branch-exists", RunnerRefusal.branchExists.sentence),
            ("code: worktree-exists", RunnerRefusal.worktreeExists.sentence),
            ("code: repository-locked", "Git is busy with something else in this repository. Try again when it’s done."),
            ("code: workspaces-exist", RunnerRefusal.worktreesExist.sentence),
            ("code: path-not-allowed", RunnerRefusal.pathNotAllowed.sentence),
            ("code: sensitive-root", RunnerRefusal.sensitiveRoot.sentence),
            ("code: base-unresolvable", RunnerRefusal.baseUnresolvable.sentence),
            ("code: confirmation-required", "The runner needs you to confirm this first."),
            ("code: tmux-unavailable", RunnerRefusal.tmuxUnavailable.sentence),
            ("code: capability-unsupported", RunnerRefusal.capabilityUnsupported.sentence),
            (
                "code: version-incompatible",
                "This runner’s Far Cooler and this app are different versions. Update the older one, then try again."
            ),
            ("code: scope-denied", RunnerRefusal.scopeDenied.sentence),
            ("code: auth-required", RunnerRefusal.authRequired.sentence),
            ("code: resource-conflict\nwhat: not_held", RunnerRefusal.resourceConflict.sentence),
            ("code: host-offline", "The runner is offline."),
            ("code: agent-not-connected", RunnerRefusal.agentNotConnected.sentence),
            ("code: agent-stopped", RunnerRefusal.agentStopped.sentence),
            ("code: attachment-limit", "That’s more than the runner takes at once."),
            ("code: diff-too-large", "The changes are too large to show."),
            ("code: diff-unsupported", "The runner can’t show these changes."),
            ("code: pr-state-unavailable", "The runner couldn’t read the pull request."),
            ("code: dispatch-unknown", "The runner didn’t say whether it went through. Check before trying again."),
            ("code: output-gap", "The runner fell behind. Try again."),
            ("code: client-too-slow", "The runner fell behind. Try again."),
            ("code: operation-failed", "The runner tried and it didn’t work. Try again."),
            ("code: invalid-argument", RunnerRefusal.invalidArgument.sentence),
            ("code: tmux-timed-out", RunnerRefusal.tmuxTimedOut.sentence),
            ("code: idempotency-mismatch", app),
            ("code: unspecified", neutral),
            ("code: unrecognized", neutral),
            ("code: some-word-from-a-newer-runner", neutral),
            ("ssh: connect to host runner port 22: Connection refused", reach),
            ("ssh: Could not resolve hostname runner: nodename nor servname provided", reach),
            ("could not reach the daemon: No such file or directory (os error 2)", reach),
            ("Connection closed by 10.0.0.2 port 22", reach),
            ("error: unexpected argument '--frobnicate' found", neutral),
            ("no layout matching \"@9\"", neutral),
        ]
        for (stderr, reason) in cases {
            #expect(ActionCopy.reason("error: whatever the daemon said\n" + stderr) == reason, "\(stderr)")
        }
        #expect(
            ActionCopy.sentence(.close, subject: "“agent”", message: "ssh: Could not resolve hostname runner")
                == "Couldn’t close “agent”. Check that the runner is reachable, then try again.")
    }

    /// A Dismiss that lost a race says so, by the runner's `what:` word on
    /// the `resource-conflict` code, and not "a problem in the app"
    /// (`invalid-argument`) or "try again". Fails with the `not_lost` branch
    /// removed from `ActionCopy.reason`.
    @Test func aDismissThatLostTheRaceSaysTheTerminalMovedOn() {
        let raced = "error: terminal isn’t lost anymore\ncode: resource-conflict\nwhat: not_lost"
        #expect(
            ActionCopy.sentence(.dismissLost, subject: "“agent”", message: raced)
                == "Couldn’t dismiss “agent”. It was already restarted or dismissed.")
        // Any other conflict keeps its own sentence.
        #expect(
            ActionCopy.reason("error: stale\ncode: resource-conflict")
                == RunnerRefusal.resourceConflict.sentence)
    }

    /// A Close whose stop worked, where the refresh after the stop reaped
    /// the record first: the remove finds nothing to remove, which is what
    /// Close asked for. The CLI says so as `code: not-found` (its
    /// `a_resolve_miss_carries_not_found_under_json`); without the word this
    /// showed "Couldn’t close … Check that the runner is reachable".
    @Test func aCloseRacingTheReapSucceeds() async {
        let runner = Runner()
        runner.fails["terminal remove"] = "error: no terminal matching \"t1\"\ncode: not-found"
        let outcomes = ActionOutcomes()

        await outcomes.perform(Self.key(.close, "t-1"), subject: "“agent”", on: client(runner)) {
            await $0.stop(terminal: "t1")
            await $0.removeTerminal("t1")
        }

        #expect(runner.calls.contains { $0.starts(with: ["terminal", "remove"]) })
        #expect(outcomes.shown.isEmpty, "\(outcomes.shown)")
    }

    /// The banners show the latest few; the rest wait behind Show All.
    @Test func theBannersAreCapped() {
        let outcomes = ActionOutcomes()
        for n in 1...5 { outcomes.settle(Self.key(.stop, "t-\(n)"), failure: "Couldn’t stop \(n).") }
        #expect(outcomes.visible.map(\.sentence) == ["Couldn’t stop 3.", "Couldn’t stop 4.", "Couldn’t stop 5."])
        #expect(outcomes.hiddenCount == 2)
        outcomes.expanded = true
        #expect(outcomes.visible.count == 5 && outcomes.hiddenCount == 0)
        outcomes.dismissAll()
        #expect(outcomes.shown.isEmpty && !outcomes.expanded)
    }

    /// A result goes with what it's about: a terminal closed elsewhere, a
    /// worktree removed. And a Close that worked settles a Stop refused on
    /// the same terminal a moment before.
    @Test func aResultGoesWithItsTarget() throws {
        let json = #"""
            {"runtime_healthy":true,"live_panes":1,"worktrees":[{"id":"w-1","short":"w1","task":"fix-it",
              "branch":"b","worktree":"/tmp/w","state":"active",
              "terminals":[{"id":"t-1","short":"t1","title":"zsh","preset":"shell","state":"running","epoch":0}]}]}
            """#
        let fleet = try JSONDecoder().decode(Fleet.self, from: Data(json.utf8))
        let outcomes = ActionOutcomes()
        outcomes.settle(Self.key(.stop, "t-1"), failure: "Couldn’t stop “one”.")
        outcomes.settle(Self.key(.restart, "t-2"), failure: "Couldn’t restart “two”.")
        outcomes.settle(Self.key(.hide, "w-1"), failure: "Couldn’t hide “fix it”.")
        outcomes.settle(Self.key(.hide, "w-9"), failure: "Couldn’t hide “gone”.")
        outcomes.settle(Self.key(.resumeBranch, "repo b"), failure: "Couldn’t pick up “b”.")

        outcomes.prune(in: fleet)
        #expect(outcomes.shown.map(\.key) == [Self.key(.stop, "t-1"), Self.key(.hide, "w-1"), Self.key(.resumeBranch, "repo b")])

        outcomes.settle(Self.key(.close, "t-1"), failure: nil)
        outcomes.settle(Self.key(.unhide, "w-1"), failure: nil)
        #expect(outcomes.shown.map(\.key) == [Self.key(.resumeBranch, "repo b")])
    }

    /// Navigation takes down the notice and nothing else: a refused move or
    /// orchestrator start is an action's result, with an action's lifetime.
    @Test func navigationLeavesActionResults() {
        let outcomes = ActionOutcomes()
        outcomes.settle(Self.key(.move, "w-1"), failure: "“fix it” or Billing isn’t on this runner anymore.")
        outcomes.settle(Self.key(.startOrchestrator, "ws-1"), failure: "Couldn’t start the orchestrator for Billing.")
        outcomes.notice = "Select a workspace first."
        outcomes.clearNotice()
        #expect(outcomes.shown.map(\.key.verb) == [.move, .startOrchestrator])
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
