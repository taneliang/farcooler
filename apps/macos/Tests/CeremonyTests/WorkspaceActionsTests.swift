import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// What a workspace offers from the sidebar, and what its notifications say.
///
/// The views that draw these are verified by looking; what is here is the
/// half looking can't check: which menu items a workspace offers in which
/// state, which sentence a refusal becomes, whether the charter item can be
/// pressed, and which words a notification leads with.
@MainActor
struct WorkspaceActionsTests {
    private static let billing = WorkspaceSummary(
        id: "ws-billing", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1,
        repository: "repo", charter: "/Users/me/.farcooler/workspaces/billing/CHARTER.md")
    private static let main = WorkspaceSummary(
        id: "ws-main", name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: "repo")

    private static func terminal(
        title: String = "claude", workspace: String? = nil, role: String? = "agent",
        activity: String? = "blocked"
    ) -> Terminal {
        var t = Terminal(id: "t1", short: "t1", title: title, preset: "claude", state: "running", epoch: 0)
        t.workspace = workspace
        t.role = role
        t.activity = activity
        return t
    }

    private static func worktree(_ task: String, workspace: String?) -> Worktree {
        var w = Worktree(
            id: task, short: task, task: task, branch: "feat/\(task)", repository: "overnight",
            host: "", path: "/tmp/\(task)", state: "active", terminals: [])
        w.workspace = workspace
        return w
    }

    // MARK: - The workspace menu

    @Test("A workspace with no orchestrator offers to start one")
    func noOrchestratorOffersStart() {
        #expect(
            WorkspaceMenu.items(hasBoard: true, hasOrchestrator: false)
                == [.showBoard, .startOrchestrator, .showCharter])
    }

    @Test("A workspace with an orchestrator offers to replace it, never to start a second")
    func anOrchestratorOffersReplace() {
        #expect(
            WorkspaceMenu.items(hasBoard: true, hasOrchestrator: true)
                == [.showBoard, .replaceOrchestrator, .showCharter])
    }

    /// A runner without `tasks` draws no Board row, so the menu can't lead
    /// to one either.
    @Test("No board, no Show Board")
    func noBoardNoShowBoard() {
        #expect(
            WorkspaceMenu.items(hasBoard: false, hasOrchestrator: false)
                == [.startOrchestrator, .showCharter])
    }

    @Test("Menu items are title case")
    func titles() {
        #expect(WorkspaceMenu.Item.showBoard.title == "Show Board")
        #expect(WorkspaceMenu.Item.startOrchestrator.title == "Start Orchestrator")
        #expect(WorkspaceMenu.Item.replaceOrchestrator.title == "Replace Orchestrator")
        #expect(WorkspaceMenu.Item.showCharter.title == "Show Charter")
        #expect(OrchestratorHarness.allCases.map(\.title) == ["Claude", "Codex", "Cursor"])
        #expect(OrchestratorHarness.allCases.map(\.rawValue) == ["claude", "codex", "cursor"])
    }

    // MARK: - Show Charter

    @Test("This Mac's charter opens")
    func aLocalCharterOpens() throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-charter-\(UUID().uuidString).md")
        try Data("# Billing".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        let billing = WorkspaceSummary(
            id: "ws-billing", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1,
            repository: "repo", charter: file.path)
        #expect(CharterAccess.of(billing, host: "") == .open(URL(fileURLWithPath: file.path)))
    }

    /// The runner sends the path before the file exists: Main has none until
    /// the repository has a manager file, and a new workspace's is written by
    /// its orchestrator. Opening it would fail, so the item says why instead
    /// — and nothing makes the file, since an empty charter would skip the
    /// orchestrator's interview.
    @Test("A charter not written yet is disabled, and says so")
    func aCharterNotWrittenYetIsDisabled() {
        let path = "/nonexistent/fc-\(UUID().uuidString)/charter.md"
        let main = WorkspaceSummary(
            id: "ws-main", name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0,
            repository: "repo", charter: path)
        #expect(
            CharterAccess.of(main, host: "")
                == .unavailable("No charter yet. The orchestrator writes one when you first talk to it."))
        #expect(!FileManager.default.fileExists(atPath: path), "nothing wrote the charter")
    }

    /// The path is on the runner's disk. The same path on this Mac is
    /// another file, or none.
    @Test("Another runner's charter is disabled, and says why")
    func aRemoteCharterIsDisabled() {
        guard case .unavailable(let why) = CharterAccess.of(Self.billing, host: "build-box") else {
            Issue.record("a remote charter was offered")
            return
        }
        #expect(why.contains("build-box"))
    }

    @Test("A charter the runner didn't locate is disabled, and says why")
    func anUnknownCharterIsDisabled() {
        guard case .unavailable(let why) = CharterAccess.of(Self.main, host: "") else {
            Issue.record("a charter with no path was offered")
            return
        }
        #expect(!why.isEmpty)
    }

    @Test("The charter path survives the decode, and an empty one is none")
    func theCharterDecodes() throws {
        let json = Data(
            #"{"id":"w","name":"Billing","task_prefix":"bil","is_main":false,"ordinal":1,"charter":"/c.md"}"#.utf8)
        #expect(try JSONDecoder().decode(WorkspaceSummary.self, from: json).charter == "/c.md")
        let empty = Data(#"{"id":"w","charter":""}"#.utf8)
        #expect(try JSONDecoder().decode(WorkspaceSummary.self, from: empty).charter == nil)
        let older = Data(#"{"id":"w"}"#.utf8)
        #expect(try JSONDecoder().decode(WorkspaceSummary.self, from: older).charter == nil)
    }

    // MARK: - Starting an orchestrator: what a refusal says

    @Test("The start runs the CLI's own verb, by workspace id")
    func theStartArguments() {
        #expect(
            DaemonClient.startOrchestratorArguments(Self.billing, harness: .codex, replace: false)
                == ["workspace", "start-orchestrator", "ws-billing", "--harness", "codex", "--json"])
        #expect(
            DaemonClient.startOrchestratorArguments(Self.billing, harness: .claude, replace: true)
                == ["workspace", "start-orchestrator", "ws-billing", "--harness", "claude", "--replace", "--json"])
    }

    @Test("Each refusal is a sentence of the app's, never the CLI's")
    func refusalsAreSentences() {
        func said(_ stderr: String?, replace: Bool = false) -> String {
            DaemonClient.orchestratorRefusal(stderr, workspace: Self.billing, replace: replace)
        }
        // The CLI's own sentences, read from its source, so a rewording
        // there turns this red rather than quietly becoming "a problem in the
        // app". See `cliSaid`.
        let taken = "error: \(Self.cliSaid("orchestrator_taken"))\ncode: invalid-argument"
        #expect(said(taken) == "Billing already has an orchestrator. Choose Replace Orchestrator to start a new one.")
        let home = "error: \(Self.cliSaid("orchestrator_home"))\ncode: invalid-argument"
        #expect(said(home) == "The runner couldn’t make Billing’s folder, so no orchestrator started.")
        // A workspace deleted since the sidebar drew it: the CLI can't
        // resolve its id, so no daemon answers and there's no code line.
        #expect(said("error: \(Self.cliNoMatch("workspace", Self.billing.id))") == "Billing isn’t on this runner anymore.")
        #expect(said("error: \(Self.cliNoMatch("workspace", Self.billing.id))", replace: true) == "Billing isn’t on this runner anymore.")
        #expect(said("error: gone\ncode: not-found") == "Billing isn’t on this runner anymore.")
        #expect(
            said("error: x\ncode: capability-unsupported")
                == "This runner’s Far Cooler is too old to start an orchestrator. Update it there, then try again.")
        #expect(
            said("error: x\ncode: scope-denied")
                == "This runner lets Far Cooler see its workspaces but not change them.")
        #expect(said("error: x\ncode: resource-conflict") == "Billing changed while its orchestrator was starting. Try again.")
        #expect(
            said("error: x\ncode: internal")
                == "This runner couldn’t start an orchestrator for Billing. That’s a problem in the app, not in anything you did.")
        #expect(
            said("ssh: connect to host build-box port 22: Connection refused")
                == "Couldn’t start an orchestrator for Billing. Check that the runner is reachable, then try again.")
        #expect(
            said(nil, replace: true)
                == "Couldn’t replace Billing’s orchestrator. Check that the runner is reachable, then try again.")
        // Nothing the CLI said is repeated to the person.
        for stderr in [taken, home, "error: x\ncode: internal", "ssh: Connection refused"] {
            #expect(!said(stderr).contains("error:"))
            #expect(!said(stderr).contains("code:"))
        }
    }

    // MARK: - The CLI's words

    private static let repoRoot = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // CeremonyTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .deletingLastPathComponent()  // apps
        .deletingLastPathComponent()  // repo root

    /// What `farcooler workspace start-orchestrator` prints for the runner's
    /// `what` word: `said_about` in crates/cli/src/tasks.rs, read out of the
    /// source line `"<what>" => "<sentence>",`. An empty string, which no
    /// mapping matches, if the line is gone.
    private static func cliSaid(_ what: String) -> String {
        let source = (try? String(
            contentsOf: repoRoot.appendingPathComponent("crates/cli/src/tasks.rs"), encoding: .utf8)) ?? ""
        let key = "\"\(what)\" => \""
        for line in source.split(separator: "\n") {
            let line = line.trimmingCharacters(in: .whitespaces)
            guard line.hasPrefix(key), line.hasSuffix("\",") else { continue }
            return String(line.dropFirst(key.count).dropLast(2))
        }
        Issue.record("crates/cli/src/tasks.rs no longer says anything for \(what)")
        return ""
    }

    /// What the CLI's `resolve` prints for an id it can't find, from its own
    /// format string in crates/cli/src/main.rs.
    private static func cliNoMatch(_ kind: String, _ given: String) -> String {
        let source = (try? String(
            contentsOf: repoRoot.appendingPathComponent("crates/cli/src/main.rs"), encoding: .utf8)) ?? ""
        let format = #"0 => Err(format!("no {kind} matching {prefix:?}")),"#
        guard source.contains(format) else {
            Issue.record("crates/cli/src/main.rs no longer says \"no {kind} matching\"")
            return ""
        }
        return "no \(kind) matching \"\(given)\""
    }

    // MARK: - Asking before replacing, and starting once

    /// The header's Replace only ever asks; it's the confirmation that sends
    /// `--replace`. Start goes straight to the runner.
    @Test("Replace asks first; Start doesn't")
    func replaceAsksFirst() {
        for harness in OrchestratorHarness.allCases {
            #expect(OrchestratorRequest(harness: harness, replace: true) == .confirmReplace(harness))
            #expect(OrchestratorRequest(harness: harness, replace: false) == .start(harness))
        }
    }

    /// A second click while the runner is still answering the first is
    /// dropped; once it answers, the workspace can be started again.
    @Test("A second start while one is in flight is dropped")
    func aSecondStartIsDropped() {
        var starts = OrchestratorStarts()
        let first = starts.begin(Self.billing, host: "")
        let second = starts.begin(Self.billing, host: "")
        #expect(first)
        #expect(!second)
        #expect(starts.isStarting(Self.billing, host: ""))
        // Another runner's workspace of the same id is another workspace.
        let elsewhere = starts.begin(Self.billing, host: "build-box")
        #expect(elsewhere)
        starts.end(Self.billing, host: "")
        #expect(!starts.isStarting(Self.billing, host: ""))
        let again = starts.begin(Self.billing, host: "")
        #expect(again)
    }

    // MARK: - Notifications name the workspace

    @Test("An agent's notification names its workspace, then its worktree")
    func anAgentNamesItsWorkspace() {
        let place = Notifier.place(
            of: Self.terminal(workspace: "ws-billing"),
            in: Self.worktree("fc-3-webhooks", workspace: "ws-billing"),
            workspaces: [Self.main, Self.billing])
        #expect(place == "Billing · fc-3-webhooks")
    }

    /// Its checkout is the main one, which every orchestrator shares, so it
    /// says nothing about which one this is.
    @Test("An orchestrator's notification names its workspace alone")
    func anOrchestratorNamesItsWorkspace() {
        let orchestrator = Self.terminal(title: "orchestrator", workspace: "ws-billing", role: "orchestrator")
        let checkout = Self.worktree("overnight", workspace: "ws-main")
        #expect(Notifier.place(of: orchestrator, in: checkout, workspaces: [Self.main, Self.billing]) == "Billing")
        let words = Notifier.words(for: orchestrator, place: "Billing")
        #expect(words?.title == "Orchestrator needs you")
        #expect(words?.body == "Billing — Waiting for your answer")
    }

    @Test("An unclaimed worktree, or a runner without workspaces, names the worktree")
    func noWorkspaceNamesTheWorktree() {
        let wt = Self.worktree("fc-3-webhooks", workspace: nil)
        #expect(Notifier.place(of: Self.terminal(), in: wt, workspaces: [Self.billing]) == "fc-3-webhooks")
        #expect(Notifier.place(of: Self.terminal(), in: wt, workspaces: nil) == "fc-3-webhooks")
        // A workspace this runner didn't list is not named by its id.
        let stray = Self.worktree("fc-3-webhooks", workspace: "ws-gone")
        #expect(Notifier.place(of: Self.terminal(), in: stray, workspaces: [Self.billing]) == "fc-3-webhooks")
    }

    @Test("An agent's notification is titled by the agent, and leads its body with the place")
    func anAgentsWords() {
        let words = Notifier.words(for: Self.terminal(title: "claude"), place: "Billing · fc-3-webhooks")
        #expect(words?.title == "claude needs you")
        #expect(words?.body == "Billing · fc-3-webhooks — Waiting for your answer")
        #expect(Notifier.words(for: Self.terminal(activity: "working"), place: "x") == nil)
    }
}

/// A notification that arrives while Far Cooler is frontmost: shown unless
/// its terminal is on screen and somebody is there.
@MainActor
struct NotifierPresentationTests {
    private static func presence(idle: TimeInterval) -> Presence {
        Presence(
            appActive: { true }, screenAwake: { true }, sessionUnlocked: { true },
            secondsSinceInput: { idle })
    }

    @Test("A frontmost app shows the banner and plays the sound for a pane not on screen")
    func showsForAPaneNotOnScreen() {
        let here = Self.presence(idle: 1)
        #expect(Notifier.presentation(terminalID: "t2", watching: ["t1"], presence: here) == [.banner, .list, .sound])
        #expect(Notifier.presentation(terminalID: "t2", watching: [], presence: here) == [.banner, .list, .sound])
    }

    @Test("Two windows' visible sets union, and neither overwrites the other")
    func twoWindowsUnion() {
        let a = UUID(), b = UUID()
        defer { Notifier.shared.closeWindow(a); Notifier.shared.closeWindow(b) }
        Notifier.shared.setWatching(["ta1", "ta2"], window: a)
        Notifier.shared.setWatching(["tb1"], window: b)
        #expect(Notifier.shared.watching.isSuperset(of: ["ta1", "ta2", "tb1"]))
        // The last writer no longer wins: a redraw of A leaves B's pane in.
        Notifier.shared.setWatching(["ta2"], window: a)
        #expect(Notifier.shared.watching.isSuperset(of: ["ta2", "tb1"]))
        #expect(!Notifier.shared.watching.contains("ta1"))
    }

    @Test("Closing a window removes its terminals and only its terminals")
    func closingAWindowDropsItsTerminals() {
        let a = UUID(), b = UUID()
        defer { Notifier.shared.closeWindow(a); Notifier.shared.closeWindow(b) }
        Notifier.shared.setWatching(["cta"], window: a)
        Notifier.shared.setWatching(["ctb"], window: b)
        Notifier.shared.closeWindow(a)
        #expect(!Notifier.shared.watching.contains("cta"))
        #expect(Notifier.shared.watching.contains("ctb"))
    }

    @Test("A pane on screen gets no banner, only the list entry")
    func silentForAPaneOnScreen() {
        #expect(Notifier.presentation(terminalID: "t1", watching: ["t1", "t3"], presence: Self.presence(idle: 1)) == [.list])
    }

    @Test("A pane on screen banners once the person has been idle a minute")
    func bannersForAPaneOnScreenWhenIdle() {
        let idle = Self.presence(idle: Presence.idleLimit + 1)
        #expect(Notifier.presentation(terminalID: "t1", watching: ["t1"], presence: idle) == [.banner, .list, .sound])
    }
}
