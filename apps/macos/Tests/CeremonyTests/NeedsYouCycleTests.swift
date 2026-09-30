import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Needs You on the Mac: hearing that the list moved, and walking it.
@MainActor
struct NeedsYouCycleTests {
    /// The CLI's `events` prints `needs_you_changed` as `{"kind":"needs_you"}`
    /// (`event_json` in crates/cli/src/main.rs). A line the stream decodes
    /// into nothing is a list the window never re-reads: an answered decision
    /// changes no terminal, so nothing else would bring it back.
    @Test("The needs_you event line re-reads the list")
    func theNeedsYouEventLineReReadsTheList() {
        var heard = 0
        EventStream.dispatch(Data(#"{"kind":"needs_you"}"#.utf8), decoder: JSONDecoder(), onNeedsYou: { heard += 1 })
        #expect(heard == 1)
        EventStream.dispatch(Data(#"{"kind":"fleet"}"#.utf8), decoder: JSONDecoder(), onNeedsYou: { heard += 1 })
        #expect(heard == 1, "a fleet line isn't news about the list")
    }

    /// `worktree list --json` carries each worktree's open tasks, and a
    /// pane with no task of its own is shown under its worktree's one open
    /// task: `TaskLink`'s rule, over this app's own types.
    @Test("A worktree's open tasks decode, and name a shell's task")
    func aWorktreesOpenTasksDecodeAndNameAShellsTask() throws {
        let json = #"""
            {"runtime_healthy":true,"live_panes":1,"worktrees":[{"id":"w-1","short":"w1","task":"fc-3-webhooks",
              "branch":"b","worktree":"/tmp/w","state":"active","open_tasks":[{"id":"t-9","key":"bil-9",
              "title":"Invoice PDF export","status":"in_progress"}],
              "terminals":[{"id":"s-1","short":"s1","title":"zsh","preset":"shell","state":"running","epoch":0}]}]}
            """#
        let fleet = try JSONDecoder().decode(Fleet.self, from: Data(json.utf8))
        let worktree = try #require(fleet.worktrees.first)
        #expect(worktree.openTasks?.map(\.key) == ["bil-9"])
        let shell = try #require(worktree.terminals.first)
        #expect(TaskLink.task(of: shell, in: worktree) == "t-9")
        // Find finds a task by the worktree it's in, by key or title.
        let byKey = worktree.matches("bil-9"), byTitle = worktree.matches("invoice"), neither = worktree.matches("stripe")
        #expect(byKey && byTitle && !neither)
    }

    // MARK: - ⌃⌘N

    private static let billing = "0198f2c0-0000-7000-8000-0000000000dd"
    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"

    private static func item(_ id: String, _ kind: NeedsYouKind, rank: UInt32, task: String? = nil) -> NeedsYouItem {
        NeedsYouItem(
            id: id, kind: kind, rank: rank, since: nil, workspaceID: billing, workspaceName: "Billing",
            repositoryID: repo,
            task: task.map { NeedsYouTask(id: $0, key: "bil-7", title: "Approve the schema", status: "needs_decision") },
            question: "Postgres or SQLite?")
    }

    /// A decision is on no terminal, so the old walk over terminals never
    /// reached one. The walk is over items now, and a decision opens its
    /// task's column in its workspace.
    @Test("⌃⌘N reaches a decision")
    func controlCommandNReachesADecision() {
        // A blocked agent is in the fleet too; under the old walk over
        // terminals it was the only stop, and the decision was never reached.
        var blocked = Terminal(id: "blocked", short: "blocked", title: "", preset: "codex", state: "running", epoch: 0)
        blocked.activity = "blocked"
        let lane = Worktree(
            id: "w-1", short: "w1", task: "lane", branch: "b", repository: "overnight", host: "",
            path: "/tmp/lane", state: "active", terminals: [blocked], repositoryID: Self.repo)
        let fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [lane], branchPrefix: nil)
        let decision = Self.item("decision:t-7", .decision, rank: 300, task: "t-7")
        let review = Self.item("review:t-8", .review, rank: 400, task: "t-8")
        let items = NeedsYou.merge(["": [review, decision]])

        let first = NeedsYouNavigation.step(lastOpened: nil, items: items, fleet: fleet, showing: nil)
        #expect(first?.item.itemID == "decision:t-7")
        #expect(first?.landing == .workspace(host: "", workspace: Self.billing, focus: .task("t-7")))
        // On from the one it opened, while the window still shows it.
        let second = NeedsYouNavigation.step(
            lastOpened: first?.item.key, items: items, fleet: fleet, showing: first?.landing)
        #expect(second?.item.itemID == "review:t-8")
        // And from the top once you've gone somewhere else.
        let elsewhere = NeedsYouNavigation.step(
            lastOpened: first?.item.key, items: items, fleet: fleet,
            showing: .looseWorktree(host: "", worktree: "w-1", terminal: nil))
        #expect(elsewhere?.item.itemID == "decision:t-7")
    }

    /// A finished agent wanted attention under the old rule (`done` did),
    /// and ⌃⌘N stopped at it. It's not an item (ruling 1): with nothing else
    /// waiting, ⌃⌘N opens nothing, and with a block elsewhere it goes there.
    @Test("⌃⌘N skips a finished agent")
    func controlCommandNSkipsAFinishedAgent() {
        var done = Terminal(id: "done", short: "done", title: "", preset: "claude", state: "running", epoch: 0)
        done.activity = "done"
        #expect(done.agent.wantsAttention, "the old rule stopped here")
        let lane = Worktree(
            id: "w-1", short: "w1", task: "lane", branch: "b", repository: "overnight", host: "",
            path: "/tmp/lane", state: "active", terminals: [done], repositoryID: Self.repo)
        let fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [lane], branchPrefix: nil)
        // The Mac's own derivation, for a runner too old to send a list.
        let derived = NeedsYou.merge(["": NeedsYou.derived(fromTerminals: DaemonClient.olderPanes(in: [lane]))])
        #expect(derived.isEmpty)
        #expect(NeedsYouNavigation.step(lastOpened: nil, items: derived, fleet: fleet, showing: nil) == nil)

        let decision = Self.item("decision:t-7", .decision, rank: 300, task: "t-7")
        let items = NeedsYou.merge(["": [decision]])
        let opened = NeedsYouNavigation.step(lastOpened: nil, items: items, fleet: fleet, showing: nil)
        #expect(opened?.item.itemID == "decision:t-7")
        #expect(opened?.landing != .looseWorktree(host: "", worktree: "w-1", terminal: "done"))
        #expect(opened?.landing != .workspace(host: "", workspace: Self.repo, focus: .worktree("w-1", terminal: "done")))
    }
}
