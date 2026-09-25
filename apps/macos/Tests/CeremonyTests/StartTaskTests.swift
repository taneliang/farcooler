import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Starting a task from the ⌘N panel, against a runner that is only a stub.
///
/// The failures these guard are invisible by looking at code that seems to
/// work: `startTask` used to hold the window for up to a minute while it
/// waited for the agent to look idle, gave up without a word when a trust
/// screen made it look blocked, and lost a task outright when a create failed.
/// The stub's agent never becomes idle unless a test says so.
@MainActor
struct StartTaskTests {
    /// A runner in miniature: what the CLI would print, and every command it
    /// was asked to run.
    @MainActor
    final class Runner {
        var calls: [[String]] = []
        var capabilities: [String]
        var workspaceMade = false
        var terminalMade = false
        var activity: String?
        /// Worktree directories already on the runner, beside the main checkout.
        var existing: [String] = []
        /// Branches git already has, as `workspace branches` lists them.
        var branches: [(name: String, remote: String?)] = []
        var branchPrefix = ""
        /// A workspace somebody else's create made, listed FIRST — what a
        /// before-and-after diff of the fleet would wrongly take for ours.
        var anotherNewWorkspace = false
        var workspaceCreateFails: String?
        var terminalCreateFails: String?
        var branchListFails = false
        var listFails = false

        init(capabilities: [String]) { self.capabilities = capabilities }

        func answer(_ args: [String]) -> (data: Data?, message: String?) {
            calls.append(args)
            let words = args.filter { $0 != "--json" }
            switch Array(words.prefix(2)) {
            case ["workspace", "create"]:
                if let failure = workspaceCreateFails { return (nil, failure) }
                workspaceMade = true
                return (json(["id": "w-new", "short": "wnew"]), nil)
            case ["terminal", "create"]:
                if let failure = terminalCreateFails { return (nil, failure) }
                terminalMade = true
                return (json(["id": "t-new", "short": "tnew"]), nil)
            case ["workspace", "branches"]:
                if branchListFails { return (nil, "error: operation failed") }
                let list = branches.map { branch -> [String: Any] in
                    var entry: [String: Any] = ["name": branch.name, "local": branch.remote == nil]
                    if let remote = branch.remote { entry["remote"] = remote }
                    return entry
                }
                return (json(["branches": list]), nil)
            case ["workspace", "list"]:
                if listFails { return (nil, "error: could not reach the daemon") }
                return (fleet(), nil)
            case ["status"]:
                return (
                    json([
                        "daemonVersion": "0.1.0+test", "buildsMatch": true, "platform": "macos",
                        "capabilities": capabilities,
                    ]), nil
                )
            default: return (Data(), nil)
            }
        }

        private func json(_ object: [String: Any]) -> Data? {
            try? JSONSerialization.data(withJSONObject: object)
        }

        private func fleet() -> Data {
            var terminals: [[String: Any]] = []
            if terminalMade {
                var terminal: [String: Any] = [
                    "id": "t-new", "short": "tnew", "title": "claude", "preset": "claude",
                    "state": "running", "epoch": 0,
                ]
                if let activity { terminal["activity"] = activity }
                terminals.append(terminal)
            }
            func workspace(_ id: String, _ directory: String, _ terminals: [[String: Any]] = [])
                -> [String: Any]
            {
                [
                    "id": id, "short": id.replacingOccurrences(of: "-", with: ""), "task": directory,
                    "branch": directory, "worktree": "/tmp/worktrees/repo/\(directory)",
                    "state": "active", "terminals": terminals,
                ]
            }
            var workspaces: [[String: Any]] = [workspace("w-main", "repo")]
            if anotherNewWorkspace { workspaces.insert(workspace("w-other", "other"), at: 0) }
            for (index, directory) in existing.enumerated() {
                workspaces.append(workspace("w-old\(index)", directory))
            }
            if workspaceMade { workspaces.append(workspace("w-new", "new", terminals)) }
            return json([
                "runtime_healthy": true, "live_panes": 0, "workspaces": workspaces,
                "branch_prefix": branchPrefix,
            ]) ?? Data()
        }

        func made(_ verb: String) -> [String]? {
            calls.first { $0.filter { $0 != "--json" }.starts(with: [verb, "create"]) }
        }
        var sent: [[String]] { calls.filter { $0.starts(with: ["terminal", "send"]) } }
        var watchingCalls: [[String]] { calls.filter { $0.starts(with: ["terminal", "watching"]) } }
    }

    /// A client wired to `runner`, connected, with the runner's build read.
    private func client(_ runner: Runner) async -> DaemonClient {
        let client = DaemonClient(target: "")
        client.commandRunnerForTesting = { args in runner.answer(args) }
        await client.refresh()
        for _ in 0..<100 where client.daemonBuild == nil {
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(client.daemonBuild != nil, "the stub's status was read")
        return client
    }

    private static let description = "Please fix the \"flaky\" reconnect test — it's $HOME"
    private static let prompting = ["workspaces", "terminals", "launch_prompt"]

    private func start(
        _ client: DaemonClient, name: String = "fix-flaky-reconnect", agent: String = "claude",
        description: String = description
    ) async -> DaemonClient.TaskStart {
        await client.startTask(project: "repo", description: description, name: name, agent: agent)
    }

    @Test func aRunnerThatTakesALaunchPromptGetsTheTaskAsOneAndNothingIsTyped() async {
        let runner = Runner(capabilities: Self.prompting)
        let client = await client(runner)

        let started = ContinuousClock.now
        let outcome = await start(client)
        let took = ContinuousClock.now - started

        #expect(outcome == .started(workspace: "w-new", terminal: "t-new"))
        #expect(took < .seconds(2), "returned once the terminal existed, not after a wait: \(took)")
        #expect(runner.made("terminal")?.contains("--prompt=\(Self.description)") == true)
        // Nothing is typed, then or later — the agent already has it. Idle
        // now, which is what the typing path waits for.
        runner.activity = "idle"
        try? await Task.sleep(for: .milliseconds(1500))
        #expect(runner.sent.isEmpty, "typed as well: \(runner.sent)")
    }

    @Test func anOlderRunnerStillGetsTheTaskTypedButTheWindowDoesNotWaitForIt() async {
        let runner = Runner(capabilities: ["workspaces", "terminals"])
        let client = await client(runner)

        let started = ContinuousClock.now
        let outcome = await start(client)
        let took = ContinuousClock.now - started

        #expect(outcome == .started(workspace: "w-new", terminal: "t-new"))
        #expect(took < .seconds(2), "the typing no longer holds the window: \(took)")
        #expect(runner.made("terminal")?.contains { $0.hasPrefix("--prompt") } == false)

        runner.activity = "idle"
        for _ in 0..<60 where runner.sent.isEmpty {
            try? await Task.sleep(for: .milliseconds(100))
        }
        #expect(runner.sent.first == ["terminal", "send", "tnew", Self.description])
    }

    @Test func anAgentThatTakesNoPromptArgumentIsTypedInstead() async {
        let runner = Runner(capabilities: Self.prompting)
        let client = await client(runner)
        _ = await start(client, agent: "aider")
        #expect(runner.made("terminal")?.contains { $0.hasPrefix("--prompt") } == false)
    }

    @Test func theWorkspaceIsTheOneItsCreateReturnedNotANewcomerInTheList() async {
        let runner = Runner(capabilities: Self.prompting)
        runner.anotherNewWorkspace = true
        let client = await client(runner)
        runner.anotherNewWorkspace = false
        let outcome = await startWithANewcomer(client, runner)
        #expect(outcome == .started(workspace: "w-new", terminal: "t-new"))
        #expect(runner.made("terminal")?.contains("wnew") == true)
    }

    /// Another workspace appears in the list during this start.
    private func startWithANewcomer(_ client: DaemonClient, _ runner: Runner) async
        -> DaemonClient.TaskStart
    {
        runner.anotherNewWorkspace = true
        return await start(client)
    }

    @Test func theWorktreeAndItsBranchAreTheShortNameNotTheDescription() async {
        let runner = Runner(capabilities: Self.prompting)
        let client = await client(runner)
        _ = await start(client)
        #expect(
            runner.made("workspace")?.filter { $0 != "--json" } == [
                "workspace", "create", "repo", "fix-flaky-reconnect", "--branch",
                "fix-flaky-reconnect", "--no-terminal",
            ])
    }

    @Test func aNameAWorktreeOrABranchAlreadyHasGetsANumber() async {
        for setUp in [
            { (r: Runner) in r.existing = ["fix-flaky-reconnect"] },
            { (r: Runner) in r.branches = [("fix-flaky-reconnect", nil)] },
            // Only on a remote: the daemon would check it OUT, starting the
            // task on somebody's old commits.
            { (r: Runner) in r.branches = [("fix-flaky-reconnect", "origin")] },
            { (r: Runner) in
                r.branchPrefix = "el/"
                r.branches = [("el/fix-flaky-reconnect", "origin")]
            },
        ] {
            let runner = Runner(capabilities: Self.prompting)
            setUp(runner)
            let client = await client(runner)
            _ = await start(client)
            let args = runner.made("workspace")?.filter { $0 != "--json" }
            #expect(args?[3] == "fix-flaky-reconnect-2", "\(args ?? [])")
            #expect(args?[5].hasSuffix("fix-flaky-reconnect-2") == true, "\(args ?? [])")
        }
    }

    @Test func aWorkspaceThatCannotBeMadeSaysSoAndMakesNoAgent() async {
        let runner = Runner(capabilities: Self.prompting)
        runner.workspaceCreateFails = "error: branch already exists"
        let client = await client(runner)
        let outcome = await start(client)
        guard case .failed(let sentence, let workspace) = outcome else {
            Issue.record("expected a failure, got \(outcome)")
            return
        }
        #expect(workspace == nil)
        #expect(!sentence.contains("error:") && !sentence.contains("already exists"), "\(sentence)")
        #expect(runner.made("terminal") == nil)
    }

    @Test func anAgentThatCannotBeStartedSaysSoAndNamesTheWorkspace() async {
        let runner = Runner(capabilities: Self.prompting)
        runner.terminalCreateFails = "error: tmux is unavailable"
        let client = await client(runner)
        let outcome = await start(client)
        guard case .failed(let sentence, let workspace) = outcome else {
            Issue.record("expected a failure, got \(outcome)")
            return
        }
        #expect(workspace == "w-new")
        #expect(sentence.contains("tmux") && !sentence.contains("error:"), "\(sentence)")
    }

    @Test func aTaskTooLongForAnAgentIsRefusedBeforeAnythingIsMade() async {
        let runner = Runner(capabilities: Self.prompting)
        let client = await client(runner)
        for description in [String(repeating: "x", count: TaskPrompt.maxBytes + 1), "a\u{0}b"] {
            let outcome = await start(client, description: description)
            guard case .failed = outcome else {
                Issue.record("expected a refusal, got \(outcome)")
                continue
            }
        }
        #expect(runner.made("workspace") == nil)
    }

    @Test func branchesThatCannotBeReadStopTheStartRatherThanRiskAnExistingBranch() async {
        let runner = Runner(capabilities: Self.prompting)
        runner.branchListFails = true
        let client = await client(runner)
        guard case .failed = await start(client) else {
            Issue.record("expected a failure")
            return
        }
        #expect(runner.made("workspace") == nil)
    }

    @Test func aPromptThatStartsWithADashIsStillOneArgument() {
        let args = DaemonClient.taskTerminalArguments(
            workspace: "w", agent: "codex", prompt: "--help me")
        #expect(args == ["terminal", "create", "w", "--preset", "codex", "--title", "codex",
                         "--prompt=--help me"])
        #expect(
            !DaemonClient.taskTerminalArguments(workspace: "w", agent: "codex", prompt: nil)
                .contains { $0.hasPrefix("--prompt") })
    }

    // MARK: - The "watching" heartbeat

    private static func present(
        app: Bool = true, awake: Bool = true, unlocked: Bool = true, idle: TimeInterval = 1
    ) -> Presence {
        Presence(
            appActive: { app }, screenAwake: { awake }, sessionUnlocked: { unlocked },
            secondsSinceInput: { idle })
    }

    private func watching(_ runner: Runner, presence: Presence) async -> [[String]] {
        let client = await client(runner)
        client.presence = presence
        client.reportWatching(["t1"])
        try? await Task.sleep(for: .milliseconds(100))
        return runner.watchingCalls
    }

    @Test func somebodyPresentClaimsThePanesOnScreen() async {
        let runner = Runner(capabilities: ["workspaces", "terminals", "watching"])
        #expect(await watching(runner, presence: Self.present()) == [["terminal", "watching", "t1"]])
    }

    @Test(arguments: ["app in the background", "display asleep", "locked or switched away",
                      "no input for a minute"])
    func nobodyPresentClaimsNothing(_ absent: String) async {
        let presence: Presence =
            switch absent {
            case "app in the background": Self.present(app: false)
            case "display asleep": Self.present(awake: false)
            case "locked or switched away": Self.present(unlocked: false)
            default: Self.present(idle: Presence.idleLimit + 1)
            }
        let runner = Runner(capabilities: ["workspaces", "terminals", "watching"])
        let calls = await watching(runner, presence: presence)
        #expect(!calls.contains { $0.contains("t1") }, "\(absent): \(calls)")
    }

    @Test func aRunnerThatIsNotConnectedIsNeverSentAHeartbeat() async {
        let runner = Runner(capabilities: ["workspaces", "terminals", "watching"])
        let client = await client(runner)
        client.presence = Self.present()
        runner.listFails = true
        await client.refresh()
        client.reportWatching(["t1"])
        try? await Task.sleep(for: .milliseconds(100))
        #expect(runner.watchingCalls.isEmpty)
    }

    @Test func nothingClaimedAndNothingToTakeBackSendsNothing() async {
        let runner = Runner(capabilities: ["workspaces", "terminals", "watching"])
        let client = await client(runner)
        client.presence = Self.present()
        client.reportWatching([])
        client.reportWatching([])
        try? await Task.sleep(for: .milliseconds(100))
        #expect(runner.watchingCalls.isEmpty)
    }
}
