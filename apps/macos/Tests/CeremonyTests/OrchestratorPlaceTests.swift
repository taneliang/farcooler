import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A terminal is shown in exactly one place (ov-63): an orchestrator in its
/// workspace's conversation column, and nowhere else, however many other
/// terminals share its worktree or its tmux window.
@MainActor
struct OrchestratorPlaceTests {
    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let main = "0198f2c0-0000-7000-8000-0000000000cc"

    /// The owner's overnight Main: claude adopted as the orchestrator in the
    /// main checkout, and `sleepnomore` in a shell beside it.
    private static func fleet() -> Fleet {
        var conductor = Terminal(id: "conductor", short: "cd", title: "claude", preset: "claude", state: "running", epoch: 0)
        conductor.role = "orchestrator"
        conductor.workspace = main
        let shell = Terminal(id: "s1", short: "s1", title: "sleepnomore", preset: "zsh", state: "running", epoch: 0)
        var checkout = Worktree(
            id: "checkout", short: "co", task: "main", branch: "main", repository: "overnight", host: "",
            path: "/tmp/co", state: "active", terminals: [conductor, shell], repositoryID: repo, workspace: main)
        checkout.is_main_checkout = true
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [checkout], branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [
            WorkspaceSummary(id: main, name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: repo, orchestrator: "conductor")
        ]
        return fleet
    }

    private static func pane(_ id: String, left: Int) -> PaneRect {
        PaneRect(id: id, short: id, title: nil, left: left, top: 0, columns: 40, rows: 24, focused: left == 0, zoomed: false)
    }

    /// Both in one tmux window, as a split made by hand leaves them.
    private static let shared = [
        PaneGroup(id: "@1", name: "", active: true, columns: 81, rows: 24, layout: "@1", panes: [pane("conductor", left: 0), pane("s1", left: 41)])
    ]

    private static var workspace: ContentView.Selection { .workspace(host: "", workspace: main, focus: nil) }

    /// The conversation column never draws the window whole while another
    /// terminal is in it: it draws the orchestrator on its own, and the
    /// app asks the runner to give it a window of its own.
    @Test("An orchestrator sharing a window is drawn alone, and broken out")
    func anOrchestratorSharingAWindowIsDrawnAloneAndBrokenOut() {
        let fleet = Self.fleet()
        let shown = WorkspaceScreen.shown(Self.workspace, in: fleet, layouts: { _, _ in Self.shared })
        #expect(!shown.contains { $0.column == .conversation }, "the conversation drew sleepnomore beside the orchestrator")
        let seat = WorkspaceScreen.pane("conductor", host: "", in: fleet)!
        #expect(WorkspaceScreen.sharesWindow(seat, layouts: Self.shared))

        // Once it has a window of its own, that window is the column's.
        let apart = [
            PaneGroup(id: "@1", name: "", active: false, columns: 80, rows: 24, layout: "@1", panes: [Self.pane("s1", left: 0)]),
            PaneGroup(id: "@2", name: "", active: true, columns: 80, rows: 24, layout: "@2", panes: [Self.pane("conductor", left: 0)]),
        ]
        #expect(!WorkspaceScreen.sharesWindow(seat, layouts: apart))
        let after = WorkspaceScreen.shown(Self.workspace, in: fleet, layouts: { _, _ in apart })
        #expect(after.first { $0.column == .conversation }?.group.terminals == ["conductor"])
        // Nothing read yet: nothing to break out.
        #expect(!WorkspaceScreen.sharesWindow(seat, layouts: nil))

        // Until then, sleepnomore opened from the main checkout doesn't draw
        // the shared window either: the checkout falls back to its cards.
        let sleep = ContentView.Selection.workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: "s1"))
        #expect(WorkspaceScreen.shown(sleep, in: fleet, layouts: { _, _ in Self.shared }).isEmpty)
        // Apart, it's its own window.
        #expect(WorkspaceScreen.shown(sleep, in: fleet, layouts: { _, _ in apart }).last?.group.terminals == ["s1"])
    }

    /// The main checkout, opened, knows which orchestrators run in it, so it
    /// can leave them out and say where they are.
    @Test("The main checkout names the orchestrators it hosts")
    func theMainCheckoutNamesTheOrchestratorsItHosts() {
        let fleet = Self.fleet()
        let checkout = fleet.worktrees[0]
        #expect(WorkspaceScreen.seated(in: checkout, fleet: fleet).map(\.workspace.name) == ["Main"])
        #expect(WorkspaceScreen.seated(in: checkout, fleet: fleet).map(\.pane.terminal.id) == ["conductor"])
        // Its own terminals, for the card view, without the orchestrator.
        #expect(WorkspaceScreen.ownTerminals(of: checkout, fleet: fleet).terminals.map(\.id) == ["s1"])

        // And says where it is instead.
        #expect(WorktreeDetail.hostedSentence(["Main"]) == "The orchestrator runs here. It’s in the Orchestrator column.")
        #expect(WorktreeDetail.hostedSentence(["Main", "Billing"]).hasPrefix("The orchestrators for Main and Billing run here."))

        // A stopped orchestrator nobody seats is still the checkout's.
        var unseated = fleet
        unseated.runnerWorkspaces[""] = [
            WorkspaceSummary(id: Self.main, name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: Self.repo)
        ]
        unseated.worktrees[0].terminals[0].state = "exited"
        #expect(WorkspaceScreen.seated(in: unseated.worktrees[0], fleet: unseated).isEmpty)
        #expect(WorkspaceScreen.ownTerminals(of: unseated.worktrees[0], fleet: unseated).terminals.count == 2)
    }

    /// An orchestrator carrying a task id is still not that task's agent: a
    /// task's column never draws it.
    @Test("A task's column never draws the orchestrator")
    func aTasksColumnNeverDrawsTheOrchestrator() {
        var fleet = Self.fleet()
        fleet.worktrees[0].terminals[0].taskId = "t-1"
        #expect(WorkspaceScreen.agents(of: "t-1", host: "", in: fleet).isEmpty)
    }
}
