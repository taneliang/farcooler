import AgentKit
import AppKit
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
        var worktreeMade = false
        var terminalMade = false
        var activity: String?
        /// What the made terminal's `preset` reads as: what it was launched
        /// as, or once tmux resolves it, what is running (`zsh` for a shell).
        var terminalPreset = "claude"
        /// Worktree directories already on the runner, beside the main checkout.
        var existing: [String] = []
        /// Branches git already has, as `worktree branches` lists them.
        var branches: [(name: String, remote: String?)] = []
        var branchPrefix = ""
        /// A worktree somebody else's create made, listed FIRST — what a
        /// before-and-after diff of the fleet would wrongly take for ours.
        var anotherNewWorktree = false
        var worktreeCreateFails: String?
        var terminalCreateFails: String?
        var branchListFails = false
        /// The runner refuses `terminal send`.
        var sendFails = false
        var listFails = false
        /// The runner refuses `task set`.
        var linkFails = false

        init(capabilities: [String]) { self.capabilities = capabilities }

        func answer(_ args: [String]) -> (data: Data?, message: String?) {
            calls.append(args)
            let words = args.filter { $0 != "--json" }
            switch Array(words.prefix(2)) {
            case ["worktree", "create"]:
                if let failure = worktreeCreateFails { return (nil, failure) }
                worktreeMade = true
                return (json(["id": "w-new", "short": "wnew"]), nil)
            case ["terminal", "create"]:
                if let failure = terminalCreateFails { return (nil, failure) }
                terminalMade = true
                return (json(["id": "t-new", "short": "tnew"]), nil)
            case ["worktree", "branches"]:
                if branchListFails { return (nil, "error: operation failed") }
                let list = branches.map { branch -> [String: Any] in
                    var entry: [String: Any] = ["name": branch.name, "local": branch.remote == nil]
                    if let remote = branch.remote { entry["remote"] = remote }
                    return entry
                }
                return (json(["branches": list]), nil)
            case ["task", "set"]:
                return linkFails ? (nil, "error: operation failed") : (json([:]), nil)
            case ["terminal", "send"]:
                return sendFails ? (nil, "error: the pane is gone") : (Data(), nil)
            case ["worktree", "list"]:
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
                    "id": "t-new", "short": "tnew", "title": "claude", "preset": terminalPreset,
                    "state": "running", "epoch": 0,
                ]
                if let activity { terminal["activity"] = activity }
                terminals.append(terminal)
            }
            func worktree(_ id: String, _ directory: String, _ terminals: [[String: Any]] = [])
                -> [String: Any]
            {
                [
                    "id": id, "short": id.replacingOccurrences(of: "-", with: ""), "task": directory,
                    "branch": directory, "worktree": "/tmp/worktrees/repo/\(directory)",
                    "state": "active", "terminals": terminals,
                ]
            }
            var worktrees: [[String: Any]] = [worktree("w-main", "repo")]
            if anotherNewWorktree { worktrees.insert(worktree("w-other", "other"), at: 0) }
            for (index, directory) in existing.enumerated() {
                worktrees.append(worktree("w-old\(index)", directory))
            }
            if worktreeMade { worktrees.append(worktree("w-new", "new", terminals)) }
            return json([
                "runtime_healthy": true, "live_panes": 0, "worktrees": worktrees,
                "branch_prefix": branchPrefix,
            ]) ?? Data()
        }

        func made(_ verb: String) -> [String]? {
            calls.first { $0.filter { $0 != "--json" }.starts(with: [verb, "create"]) }
        }
        var sent: [[String]] { calls.filter { $0.starts(with: ["terminal", "send"]) } }
        var watchingCalls: [[String]] { calls.filter { $0.starts(with: ["terminal", "watching"]) } }
        var seenCalls: [[String]] { calls.filter { $0.starts(with: ["terminal", "seen"]) } }
    }

    /// A client wired to `runner`, connected, with the runner's build read.
    private func client(_ runner: Runner, notifications: NotificationCenter = NotificationCenter())
        async -> DaemonClient
    {
        let client = DaemonClient(target: "", notifications: notifications)
        client.commandRunnerForTesting = { args in runner.answer(args) }
        // Never this Mac's own pasteboard: a typing path left running by a
        // test gives up after a minute, and would put a task on it.
        client.copyToClipboard = { _ in }
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

        #expect(outcome == .started(worktree: "w-new", terminal: "t-new", name: "fix-flaky-reconnect"))
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

        #expect(outcome == .started(worktree: "w-new", terminal: "t-new", name: "fix-flaky-reconnect"))
        #expect(took < .seconds(2), "the typing no longer holds the window: \(took)")
        #expect(runner.made("terminal")?.contains { $0.hasPrefix("--prompt") } == false)

        runner.activity = "idle"
        for _ in 0..<60 where runner.sent.isEmpty {
            try? await Task.sleep(for: .milliseconds(100))
        }
        #expect(runner.sent.first == ["terminal", "send", "tnew", Self.description])
    }

    /// Every way the typing path gives up keeps the task: it goes on the
    /// clipboard, and the window is told in a sentence for that cause —
    /// rather than the task simply not being there.
    @Test(arguments: ["asked a question", "never ready", "closed", "send refused"])
    func aTaskThatCannotBeTypedIsPutOnTheClipboardAndSaid(_ cause: String) async {
        let runner = Runner(capabilities: ["workspaces", "terminals"])
        switch cause {
        case "asked a question": runner.activity = "blocked"
        case "never ready": runner.activity = "working"
        case "send refused":
            runner.activity = "idle"
            runner.sendFails = true
        default: runner.activity = "working"
        }
        let client = await client(runner)
        client.typingPasses = 2
        var clipboard: [String] = []
        client.copyToClipboard = { clipboard.append($0) }
        var said: [String] = []
        let outcome = await client.startTask(
            project: "repo", description: Self.description, name: "fix-flaky-reconnect",
            agent: "claude", undelivered: { said.append($0) })
        #expect(outcome == .started(worktree: "w-new", terminal: "t-new", name: "fix-flaky-reconnect"))
        // Gone before the first look.
        if cause == "closed" { runner.terminalMade = false }

        for _ in 0..<50 where said.isEmpty { try? await Task.sleep(for: .milliseconds(100)) }
        #expect(clipboard == [Self.description], "\(cause)")
        let expected: TaskFailure.Undelivered =
            switch cause {
            case "asked a question": .askedFirst
            case "never ready": .neverReady
            case "closed": .gone
            default: .notTyped
            }
        #expect(said == [TaskFailure.undelivered(name: "fix-flaky-reconnect", expected)], "\(cause)")
        if cause != "send refused" { #expect(runner.sent.isEmpty, "nothing typed") }
    }

    /// Each cause says what to do next, in this app's words, naming the task.
    @Test func eachGiveUpSaysWhatToDoNext() {
        let say = { TaskFailure.undelivered(name: "fix-it", $0) }
        #expect(say(.askedFirst).contains("once you’ve answered"))
        #expect(say(.neverReady).contains("after a minute"))
        #expect(say(.gone).contains("closed before it got what you typed"))
        #expect(say(.notTyped).hasPrefix("Couldn’t type what you asked for"))
        for cause: TaskFailure.Undelivered in [.askedFirst, .neverReady, .gone, .notTyped] {
            #expect(say(cause).contains("“fix it”") && say(cause).contains("clipboard"))
        }
    }

    /// No refusal or failure sentence the ⌘N path composes calls the request
    /// a "task".
    ///
    /// A task is a card on the board now, with a row of its own above every
    /// repository's worktrees, and ⌘N puts nothing there: it makes a
    /// worktree and starts an agent. What this walks is `TaskPrompt` and
    /// `TaskFailure` — the sentences composed off the view, which are the
    /// ones a test can call. The menu item, the palette entry, the ⌘/ row,
    /// the panel's hints and `startTask`'s two refusals are literals in
    /// views and are not covered here.
    @Test func noStartFailureSentenceCallsTheRequestATask() {
        var sentences: [String] = [
            TaskPrompt.problem(String(repeating: "a", count: TaskPrompt.maxBytes + 1)) ?? "",
            TaskPrompt.problem("a\u{0}b") ?? "",
        ]
        for cause: TaskFailure.Undelivered in [.askedFirst, .neverReady, .gone, .notTyped] {
            sentences.append(TaskFailure.undelivered(name: "fix-it", cause))
        }
        for code in [
            "branch-exists", "worktree-exists", "tmux-unavailable", "tmux-timed-out",
            "capability-unsupported", "invalid-argument", "something-new",
        ] {
            sentences.append(TaskFailure.sentence(for: "error: x\ncode: \(code)"))
        }
        sentences.append(TaskFailure.sentence(for: nil))
        #expect(!sentences.contains(""), "a refusal came back empty")
        for sentence in sentences {
            let words = sentence.lowercased().split { !$0.isLetter }
            #expect(!words.contains("task") && !words.contains("tasks"), "\(sentence)")
        }
    }

    @Test func anAgentThatTakesNoPromptArgumentIsTypedInstead() async {
        let runner = Runner(capabilities: Self.prompting)
        let client = await client(runner)
        _ = await start(client, agent: "aider")
        #expect(runner.made("terminal")?.contains { $0.hasPrefix("--prompt") } == false)
    }

    @Test func theWorktreeIsTheOneItsCreateReturnedNotANewcomerInTheList() async {
        let runner = Runner(capabilities: Self.prompting)
        // Absent while the client first reads the fleet, so a before-and-after
        // guess would see TWO new worktrees during the start — and take the
        // first, which is not ours.
        let client = await client(runner)
        let outcome = await startWithANewcomer(client, runner)
        #expect(outcome == .started(worktree: "w-new", terminal: "t-new", name: "fix-flaky-reconnect"))
        #expect(runner.made("terminal")?.contains("wnew") == true)
    }

    /// Another worktree appears in the list during this start.
    private func startWithANewcomer(_ client: DaemonClient, _ runner: Runner) async
        -> DaemonClient.TaskStart
    {
        runner.anotherNewWorktree = true
        return await start(client)
    }

    @Test func theWorktreeAndItsBranchAreTheShortNameNotTheDescription() async {
        let runner = Runner(capabilities: Self.prompting)
        let client = await client(runner)
        _ = await start(client)
        #expect(
            runner.made("worktree")?.filter { $0 != "--json" } == [
                "worktree", "create", "repo", "fix-flaky-reconnect", "--branch",
                "fix-flaky-reconnect", "--no-terminal",
            ])
    }

    /// ⌘N names the workspace the window is in, so the runner claims the
    /// new worktree for it — rather than leaving it in Unclaimed until an
    /// agent's work there claims it. A runner without `workstreams` would
    /// refuse the flag, so it is sent none, and neither is a start that
    /// names no workspace.
    @Test func theCreateNamesTheWorkspaceItIsFor() async {
        let billing = "0198f2c0-0000-7000-8000-0000000000dd"
        for (capabilities, workspace, sent) in [
            (Self.prompting + ["workstreams"], billing, true),
            (Self.prompting + ["workstreams"], nil, false),
            (Self.prompting, billing, false),
        ] {
            let runner = Runner(capabilities: capabilities)
            let client = await client(runner)
            _ = await client.startTask(
                project: "repo", description: Self.description, name: "fix-flaky-reconnect",
                agent: "claude", workspace: workspace)
            let made = runner.made("worktree") ?? []
            let at = made.firstIndex(of: "--workspace")
            #expect((at != nil) == sent, "\(capabilities) \(workspace ?? "none"): \(made)")
            if let at { #expect(made[at + 1] == billing) }
        }
    }

    /// A runner that can refuse a branch a remote already has is asked to,
    /// which closes the window between reading the branch list and the create.
    /// One that can't is asked nothing it would refuse.
    @Test func theCreateIsForkOnlyWhereTheRunnerCanDoIt() async {
        for (capabilities, forkOnly) in [
            (Self.prompting + ["workspace_fork_only"], true), (Self.prompting, false),
        ] {
            let runner = Runner(capabilities: capabilities)
            let client = await client(runner)
            _ = await start(client)
            #expect(runner.made("worktree")?.contains("--fork-only") == forkOnly, "\(capabilities)")
        }
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
            // Differing only in case: one loose ref, and one directory, on a
            // Mac runner's disk.
            { (r: Runner) in r.branches = [("Fix-Flaky-Reconnect", nil)] },
            { (r: Runner) in r.existing = ["Fix-Flaky-Reconnect"] },
        ] {
            let runner = Runner(capabilities: Self.prompting)
            setUp(runner)
            let client = await client(runner)
            let outcome = await start(client)
            #expect(
                outcome == .started(worktree: "w-new", terminal: "t-new", name: "fix-flaky-reconnect-2"),
                "the name it was made under comes back")
            let args = runner.made("worktree")?.filter { $0 != "--json" }
            #expect(args?[3] == "fix-flaky-reconnect-2", "\(args ?? [])")
            #expect(args?[5].hasSuffix("fix-flaky-reconnect-2") == true, "\(args ?? [])")
        }
    }

    /// Taken by something neither list shows — a directory left under
    /// `worktrees/` that no worktree owns, a branch fetched since the list
    /// was read. Starting it again gets the next name, not the same refusal.
    @Test(arguments: ["worktree-exists", "branch-exists"])
    func aNameTheRunnerRefusedAsTakenIsNotTriedAgain(_ code: String) async {
        let runner = Runner(capabilities: Self.prompting)
        runner.worktreeCreateFails = "error: already exists\ncode: \(code)"
        let client = await client(runner)
        guard case .failed(let sentence, _) = await start(client) else {
            Issue.record("expected a refusal")
            return
        }
        #expect(sentence.contains("Start again"), "\(sentence)")

        runner.worktreeCreateFails = nil
        let outcome = await start(client)
        #expect(outcome == .started(worktree: "w-new", terminal: "t-new", name: "fix-flaky-reconnect-2"))
        let creates = runner.calls.filter { $0.filter { $0 != "--json" }.starts(with: ["worktree", "create"]) }
        #expect(creates.count == 2)
        #expect(creates.last?.contains("fix-flaky-reconnect-2") == true, "\(creates)")
    }

    @Test func aWorktreeThatCannotBeMadeSaysSoAndMakesNoAgent() async {
        let runner = Runner(capabilities: Self.prompting)
        runner.worktreeCreateFails = "error: branch already exists\ncode: branch-exists"
        let client = await client(runner)
        let outcome = await start(client)
        guard case .failed(let sentence, let made) = outcome else {
            Issue.record("expected a failure, got \(outcome)")
            return
        }
        #expect(made == nil)
        #expect(!sentence.contains("error:") && !sentence.contains("already exists"), "\(sentence)")
        #expect(runner.made("terminal") == nil)
    }

    @Test func anAgentThatCannotBeStartedSaysSoAndNamesTheWorktree() async {
        let runner = Runner(capabilities: Self.prompting)
        runner.terminalCreateFails = "error: tmux is unavailable\ncode: tmux-unavailable"
        let client = await client(runner)
        let outcome = await start(client)
        guard case .failed(let sentence, let made) = outcome else {
            Issue.record("expected a failure, got \(outcome)")
            return
        }
        #expect(made == .init(id: "w-new", name: "fix-flaky-reconnect"))
        #expect(sentence.contains("tmux") && !sentence.contains("error:"), "\(sentence)")
    }

    /// The worktree was made and its agent was not: starting again starts the
    /// agent THERE, rather than making `-2` beside it and leaving the first
    /// without one.
    @Test func aRetryStartsTheAgentInTheWorktreeTheFailedStartMade() async {
        let runner = Runner(capabilities: Self.prompting)
        runner.terminalCreateFails = "error: tmux is unavailable\ncode: tmux-unavailable"
        let client = await client(runner)
        guard case .failed(_, let made?) = await start(client) else {
            Issue.record("expected a failure that made a worktree")
            return
        }

        runner.terminalCreateFails = nil
        let outcome = await client.startTask(
            project: "repo", description: Self.description, name: made.name, agent: "claude",
            reusing: made.id)
        #expect(outcome == .started(worktree: "w-new", terminal: "t-new", name: "fix-flaky-reconnect"))
        let creates = runner.calls.filter { $0.filter { $0 != "--json" }.starts(with: ["worktree", "create"]) }
        #expect(creates.count == 1, "one worktree, made once: \(creates)")
        let terminals = runner.calls.filter { $0.filter { $0 != "--json" }.starts(with: ["terminal", "create"]) }
        #expect(terminals.count == 2 && terminals.last?.contains("wnew") == true, "\(terminals)")
    }

    /// The failed attempt's agent was in fact made — the link dropped after
    /// the runner did it — or somebody opened one there since. Starting again
    /// goes to that agent and starts no second one in the same tree.
    @Test func aRetryWhereAnAgentAlreadyRunsGoesToItAndStartsNoOther() async {
        let runner = Runner(capabilities: Self.prompting)
        let client = await client(runner)
        // Made behind the client's back: its fleet hasn't seen either yet.
        runner.worktreeMade = true
        runner.terminalMade = true
        let outcome = await client.startTask(
            project: "repo", description: Self.description, name: "fix-flaky-reconnect",
            agent: "claude", reusing: "w-new")
        #expect(outcome == .started(worktree: "w-new", terminal: "t-new", name: "fix-flaky-reconnect"))
        #expect(runner.made("terminal") == nil, "no second agent")
        #expect(runner.made("worktree") == nil, "and no second worktree")
    }

    /// A shell opened in the leftover worktree to look around is not the
    /// task's agent. A live shell pane's `preset` is the process tmux sees —
    /// `zsh`, not `shell` — and taking that for an agent selected the shell,
    /// started nothing, and lost the task without a word.
    @Test(arguments: ["zsh", "bash", "fish", "shell"])
    func aRetryWhereOnlyAShellRunsStartsTheAgent(_ shell: String) async {
        let runner = Runner(capabilities: Self.prompting)
        let client = await client(runner)
        runner.worktreeMade = true
        runner.terminalMade = true
        runner.terminalPreset = shell
        let outcome = await client.startTask(
            project: "repo", description: Self.description, name: "fix-flaky-reconnect",
            agent: "claude", reusing: "w-new")
        #expect(outcome == .started(worktree: "w-new", terminal: "t-new", name: "fix-flaky-reconnect"))
        #expect(runner.made("terminal")?.contains("--prompt=\(Self.description)") == true, "\(shell)")
        #expect(runner.made("worktree") == nil, "in the worktree it made")
    }

    /// Removed since: a start from scratch, not an agent sent nowhere.
    @Test func aRetryWhoseWorktreeIsGoneMakesANewOne() async {
        let runner = Runner(capabilities: Self.prompting)
        let client = await client(runner)
        let outcome = await client.startTask(
            project: "repo", description: Self.description, name: "fix-flaky-reconnect",
            agent: "claude", reusing: "w-removed")
        #expect(outcome == .started(worktree: "w-new", terminal: "t-new", name: "fix-flaky-reconnect"))
        #expect(runner.made("worktree") != nil)
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
        #expect(runner.made("worktree") == nil)
    }

    @Test func branchesThatCannotBeReadStopTheStartRatherThanRiskAnExistingBranch() async {
        let runner = Runner(capabilities: Self.prompting)
        runner.branchListFails = true
        let client = await client(runner)
        guard case .failed = await start(client) else {
            Issue.record("expected a failure")
            return
        }
        #expect(runner.made("worktree") == nil)
    }

    @Test func aPromptThatStartsWithADashIsStillOneArgument() {
        let args = DaemonClient.taskTerminalArguments(
            worktree: "w", agent: "codex", prompt: "--help me")
        #expect(args == ["terminal", "create", "w", "--preset", "codex", "--title", "codex",
                         "--prompt=--help me"])
        #expect(
            !DaemonClient.taskTerminalArguments(worktree: "w", agent: "codex", prompt: nil)
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
        await afterTheSentinel(client, runner)
        return runner.watchingCalls
    }

    /// Wait until a call queued AFTER the action under test has reached the
    /// runner, so a "nothing was sent" check means nothing was sent.
    ///
    /// A fixed 100 ms sleep passed just as well for a call that was on its
    /// way. `reportWatching` and `markSeen(onScreen:)` hand their call to a
    /// main-actor `Task` before they return, and the stub records it before
    /// that task first suspends, so this one, enqueued on the same actor
    /// after them, lands after theirs. `agent-prompt`, so no filter below
    /// (`sent`, `watchingCalls`, `seenCalls`) ever counts it.
    private func afterTheSentinel(_ client: DaemonClient, _ runner: Runner) async {
        let sentinel = ["terminal", "agent-prompt", "sentinel", "--", ""]
        Task { _ = await client.agentPrompt(terminal: "sentinel", text: "") }
        for _ in 0..<500 where !runner.calls.contains(sentinel) {
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(runner.calls.contains(sentinel), "the sentinel never reached the runner")
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

    /// The runner has one slot for every local client, so a second window's
    /// client saying "I show t-new" must not take away what the first window
    /// showed. What another window shows, on this runner, rides along.
    @Test func aWindowClaimsWhatEveryOtherWindowShowsToo() async {
        let runner = aFinishedAgent()
        let client = await client(runner)
        client.presence = Self.present()
        let other = UUID()
        defer { Notifier.shared.closeWindow(other) }
        Notifier.shared.setWatching(["t-new"], window: other)
        client.reportWatching([])
        try? await Task.sleep(for: .milliseconds(100))
        #expect(runner.watchingCalls == [["terminal", "watching", "t-new"]], "\(runner.watchingCalls)")

        // Once that window closes, the next call drops it.
        Notifier.shared.closeWindow(other)
        client.reportWatching([])
        try? await Task.sleep(for: .milliseconds(100))
        #expect(runner.watchingCalls.last == ["terminal", "watching"], "\(runner.watchingCalls)")
    }

    /// A minimized window, or one wholly behind others, isn't in sight
    /// (night review M5). Its panes stop silencing banners, and its runner is
    /// told it shows nothing, so a phone push for them comes back.
    @Test func aWindowOutOfSightClaimsNothing() async {
        #expect(WindowSight.inSight(miniaturized: false, occlusion: .visible))
        #expect(!WindowSight.inSight(miniaturized: true, occlusion: .visible))
        #expect(!WindowSight.inSight(miniaturized: false, occlusion: []))
        #expect(WindowSight.inSight(nil), "a window not yet placed answers as before")
        let offscreen = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 100, height: 100), styleMask: [.borderless],
            backing: .buffered, defer: false)
        offscreen.isReleasedWhenClosed = false
        #expect(!WindowSight.inSight(offscreen), "a window never ordered in is in nobody's sight")

        // A runner with no panes of its own, so what other tests' windows
        // show (`Notifier.shared`, one for the process) never rides along.
        let runner = Runner(capabilities: ["workspaces", "terminals", "watching"])
        let client = await client(runner)
        client.presence = Self.present()
        let window = UUID()
        defer { Notifier.shared.closeWindow(window) }
        Notifier.shared.setWatching(["t-sight"], window: window)
        client.reportWatching(["t1"])
        try? await Task.sleep(for: .milliseconds(100))
        #expect(runner.watchingCalls.last == ["terminal", "watching", "t1"], "\(runner.watchingCalls)")

        WindowSight.leave(window: window, clients: [client])
        try? await Task.sleep(for: .milliseconds(100))
        #expect(!Notifier.shared.watching.contains("t-sight"))
        #expect(runner.watchingCalls.last == ["terminal", "watching"], "\(runner.watchingCalls)")
    }

    /// Idle for a minute, the claim lapses; a touch of the mouse brings it
    /// back on the next tick — with no fleet event and no selection change,
    /// which is how people watch an agent in a long tool call.
    @Test func aClaimComesBackWithinATickOfThePersonReturning() async {
        let runner = Runner(capabilities: ["workspaces", "terminals", "watching"])
        let client = await client(runner)
        var idle: TimeInterval = Presence.idleLimit + 1
        client.presence = Presence(
            appActive: { true }, screenAwake: { true }, sessionUnlocked: { true },
            secondsSinceInput: { idle })
        client.reportWatching(["t1"])
        await afterTheSentinel(client, runner)
        #expect(!runner.watchingCalls.contains { $0.contains("t1") }, "nobody there yet")

        idle = 1
        for _ in 0..<40 where !runner.watchingCalls.contains(where: { $0.contains("t1") }) {
            try? await Task.sleep(for: .milliseconds(100))
        }
        #expect(runner.watchingCalls.contains { $0.contains("t1") }, "\(runner.watchingCalls)")
    }

    /// The claim goes the moment the person does — the app resigning, or
    /// `ScreenState` saying the display slept, the screen locked or the
    /// session switched away — rather than aging out on the runner while an
    /// agent finishes unannounced.
    @Test(arguments: [NSApplication.didResignActiveNotification, ScreenState.personLeft])
    func leavingGivesTheClaimBackAtOnce(_ leaving: Notification.Name) async {
        let runner = Runner(capabilities: ["workspaces", "terminals", "watching"])
        let center = NotificationCenter()
        let client = await client(runner, notifications: center)
        client.presence = Self.present()
        client.reportWatching(["t1"])
        try? await Task.sleep(for: .milliseconds(100))
        #expect(runner.watchingCalls == [["terminal", "watching", "t1"]])

        center.post(name: leaving, object: nil)
        for _ in 0..<20 where runner.watchingCalls.count < 2 {
            try? await Task.sleep(for: .milliseconds(50))
        }
        #expect(runner.watchingCalls.last == ["terminal", "watching"], "\(runner.watchingCalls)")
    }

    // MARK: - Seen

    /// A runner whose one agent, `t-new`, has finished and not been seen.
    private func aFinishedAgent(watching: Bool = true) -> Runner {
        let runner = Runner(capabilities: ["workspaces", "terminals"] + (watching ? ["watching"] : []))
        runner.worktreeMade = true
        runner.terminalMade = true
        runner.activity = "done"
        return runner
    }

    /// `done` ends when somebody sees it, so a pane on screen with nobody at
    /// the Mac — locked, asleep, walked away — stays `done`.
    @Test(arguments: [false, true])
    func aFinishedPaneIsSeenOnlyBySomebodyThere(_ present: Bool) async {
        let runner = aFinishedAgent()
        let client = await client(runner)
        client.presence = present ? Self.present() : Self.present(unlocked: false)
        let onScreen = client.fleet.worktrees.flatMap(\.terminals)
        #expect(onScreen.map(\.agent) == [.done], "the stub's agent has finished")

        client.markSeen(onScreen: onScreen)
        await afterTheSentinel(client, runner)
        #expect(runner.seenCalls == (present ? [["terminal", "seen", "tnew"]] : []))
    }

    /// It finished while nobody was there; the person comes back to the pane
    /// with a touch of the mouse — no fleet event, no click — and it is seen.
    /// On a runner without `watching` too: `done` isn't the claim's.
    @Test(arguments: [true, false])
    func aFinishedPaneIsSeenWhenThePersonComesBack(_ watching: Bool) async {
        let runner = aFinishedAgent(watching: watching)
        let client = await client(runner)
        var idle: TimeInterval = Presence.idleLimit + 1
        client.presence = Presence(
            appActive: { true }, screenAwake: { true }, sessionUnlocked: { true },
            secondsSinceInput: { idle })
        client.markSeen(onScreen: client.fleet.worktrees.flatMap(\.terminals))
        client.reportWatching(["t-new"])
        await afterTheSentinel(client, runner)
        #expect(runner.seenCalls.isEmpty, "nobody there yet")

        idle = 1
        for _ in 0..<40 where runner.seenCalls.isEmpty {
            try? await Task.sleep(for: .milliseconds(100))
        }
        #expect(runner.seenCalls == [["terminal", "seen", "tnew"]])
        // The seen above came from a renewal tick, so the clock has run. On a
        // runner without `watching` it asks nothing: no claim, and no
        // invocation spawned on that clock to be refused each time.
        if !watching {
            #expect(runner.watchingCalls.isEmpty, "\(runner.watchingCalls)")
        }
    }

    @Test func aRunnerThatIsNotConnectedIsNeverSentAHeartbeat() async {
        let runner = Runner(capabilities: ["workspaces", "terminals", "watching"])
        let client = await client(runner)
        client.presence = Self.present()
        runner.listFails = true
        await client.refresh()
        client.reportWatching(["t1"])
        await afterTheSentinel(client, runner)
        #expect(runner.watchingCalls.isEmpty)
    }

    @Test func nothingClaimedAndNothingToTakeBackSendsNothing() async {
        let runner = Runner(capabilities: ["workspaces", "terminals", "watching"])
        let client = await client(runner)
        client.presence = Self.present()
        client.reportWatching([])
        client.reportWatching([])
        await afterTheSentinel(client, runner)
        #expect(runner.watchingCalls.isEmpty)
    }
}
