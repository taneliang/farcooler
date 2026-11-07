import AgentKit
import AppKit
import Foundation
import Testing

@testable import Far_Cooler

/// The typing path and the presence heartbeat, against a runner that is only a
/// stub. The ⌘N start that used to share this file is gone (ov-951); the stub's
/// agent never becomes idle unless a test says so.
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
        /// The word the runner refuses an `--if-empty` send with, and only
        /// that send: a plain one goes through, so a flag dropped on the way
        /// shows as a send that wasn't refused.
        var ifEmptyRefusal: String?
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
                if words.contains("--if-empty") {
                    guard let word = ifEmptyRefusal else { return (Data(), nil) }
                    return (nil, "error: refused\ncode: resource-conflict\nwhat: \(word)")
                }
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
    func client(_ runner: Runner, notifications: NotificationCenter = NotificationCenter())
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

    private static let prompting = ["workspaces", "terminals", "launch_prompt"]

    /// What a started agent leaves on the runner: a worktree with an agent's
    /// terminal in it, the way the typing path finds one.
    private func madeAgent(_ runner: Runner, _ client: DaemonClient) async {
        _ = runner.answer(["--json", "worktree", "create"])
        _ = runner.answer(["--json", "terminal", "create"])
        await client.refresh()
    }

    /// A prompt the person never wrote (Suggest Themes, ov-557) is typed with
    /// `terminal send --if-empty`: one call, which the runner answers only if
    /// the agent's box is empty, so no Return of the app's own follows it, and
    /// a text that starts with a dash is still data. The plain send is
    /// unchanged.
    @Test func anOnlyIfEmptyTypeIsOneRunnerCallAndPressesNoReturnOfItsOwn() async {
        #expect(
            DaemonClient.typeArguments("-x", into: "tnew", onlyIfEmpty: true)
                == ["terminal", "send", "--json", "--if-empty", "--", "tnew", "-x"])
        #expect(
            DaemonClient.typeArguments("x", into: "tnew", onlyIfEmpty: false)
                == ["terminal", "send", "tnew", "x"])

        let runner = Runner(capabilities: Self.prompting)
        let client = await client(runner)
        await madeAgent(runner, client)
        runner.activity = "idle"
        var said: [String] = []
        client.typeWhenIdle(
            worktree: "w-new", terminal: "t-new", text: "-Suggest themes", name: "themes",
            undelivered: { said.append($0) }, onlyIfEmpty: true)
        for _ in 0..<60 where runner.sent.isEmpty {
            try? await Task.sleep(for: .milliseconds(100))
        }
        #expect(runner.sent == [["terminal", "send", "--json", "--if-empty", "--", "tnew", "-Suggest themes"]])
        // `type` returns right after the send, so a Return of the app's own
        // would already be in the calls; the wait only gives a regression
        // that sends one later a moment to show.
        try? await Task.sleep(for: .milliseconds(300))
        #expect(!runner.calls.contains { $0.starts(with: ["terminal", "send-hex"]) }, "\(runner.calls)")
        #expect(said.isEmpty)
    }

    /// The runner's refusal word reaches the window as the cause its
    /// sentence is chosen by (ov-557): text in the box, a question or dialog,
    /// a send that may have gone in, and anything else. Only the flagged send
    /// is refused by the fake, so a flag dropped between `typeWhenIdle` and the
    /// runner shows as no refusal at all.
    @Test(arguments: [
        ("draft", TaskFailure.Undelivered.notEmpty), ("prompt", .askedFirst), ("dialog", .askedFirst),
        ("unconfirmed", .mayHaveTyped), ("paste_left", .mayHaveTyped), ("typing", .notTyped),
        ("not_running", .notTyped),
    ])
    func anIfEmptySendTheRunnerRefusesIsSaidByItsWord(_ word: String, _ expected: TaskFailure.Undelivered) async {
        let runner = Runner(capabilities: Self.prompting)
        runner.ifEmptyRefusal = word
        let client = await client(runner)
        await madeAgent(runner, client)
        runner.activity = "idle"
        var causes: [TaskFailure.Undelivered] = []
        var clipboard: [String] = []
        client.copyToClipboard = { clipboard.append($0) }
        client.typeWhenIdle(
            worktree: "w-new", terminal: "t-new", text: "Suggest themes", name: "themes",
            undelivered: nil, instead: { causes.append($0) }, onlyIfEmpty: true)
        for _ in 0..<60 where causes.isEmpty {
            try? await Task.sleep(for: .milliseconds(100))
        }
        #expect(causes == [expected], "\(word)")
        #expect(runner.sent == [["terminal", "send", "--json", "--if-empty", "--", "tnew", "Suggest themes"]])
        #expect(!runner.calls.contains { $0.starts(with: ["terminal", "send-hex"]) })
        #expect(clipboard.isEmpty, "a prompt the person never wrote stays off the clipboard")
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

    // MARK: - The "watching" heartbeat

    static func present(
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
    /// after them, lands after theirs. `compose`, so no filter below
    /// (`sent`, `watchingCalls`, `seenCalls`) ever counts it.
    func afterTheSentinel(_ client: DaemonClient, _ runner: Runner) async {
        let sentinel = ["terminal", "compose", "sentinel", "--text="]
        Task { _ = await client.compose(terminal: "sentinel", text: "") }
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
        // The renewal's pause held, so "within a tick" is one tick of the
        // clock below and not three real seconds (ov-537).
        let clock = HeldClock()
        client.watchingPause = { await clock.pause() }
        client.watchingNow = { clock.now }
        client.reportWatching(["t1"])
        await afterTheSentinel(client, runner)
        #expect(!runner.watchingCalls.contains { $0.contains("t1") }, "nobody there yet")

        idle = 1
        clock.tick(DaemonClient.watchingFloor)
        await Settle.until { runner.watchingCalls.contains { $0.contains("t1") } }
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
    func aFinishedAgent(watching: Bool = true) -> Runner {
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
        let clock = HeldClock()
        client.watchingPause = { await clock.pause() }
        client.watchingNow = { clock.now }
        client.markSeen(onScreen: client.fleet.worktrees.flatMap(\.terminals))
        client.reportWatching(["t-new"])
        await afterTheSentinel(client, runner)
        #expect(runner.seenCalls.isEmpty, "nobody there yet")

        idle = 1
        clock.tick(DaemonClient.watchingFloor)
        await Settle.until { !runner.seenCalls.isEmpty }
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
