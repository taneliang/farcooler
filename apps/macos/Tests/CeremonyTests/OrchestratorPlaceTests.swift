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

    private static let apart = [
        PaneGroup(id: "@1", name: "", active: false, columns: 80, rows: 24, layout: "@1", panes: [pane("s1", left: 0)]),
        PaneGroup(id: "@2", name: "", active: true, columns: 80, rows: 24, layout: "@2", panes: [pane("conductor", left: 0)]),
    ]

    /// The column draws the orchestrator's window whole, as it always has,
    /// and names what shares it, so one click can move that out. Nothing
    /// is moved unasked.
    @Test("An orchestrator's window is drawn whole, and what shares it is named")
    func anOrchestratorsWindowIsDrawnWholeAndWhatSharesItIsNamed() {
        let fleet = Self.fleet()
        let shown = WorkspaceScreen.shown(Self.workspace, in: fleet, layouts: { _, _ in Self.shared })
        #expect(shown.first { $0.column == .conversation }?.group.terminals == ["conductor", "s1"])
        let seat = WorkspaceScreen.pane("conductor", host: "", in: fleet)!
        #expect(WorkspaceScreen.sharers(of: seat, layouts: Self.shared).map(\.id) == ["s1"])
        #expect(SharedWindowNotice.sentence("sleepnomore") == "sleepnomore shares the orchestrator’s window.")

        // Apart, or not read yet: nothing to say.
        #expect(WorkspaceScreen.sharers(of: seat, layouts: Self.apart).isEmpty)
        #expect(WorkspaceScreen.sharers(of: seat, layouts: nil).isEmpty)
        #expect(
            WorkspaceScreen.shown(Self.workspace, in: fleet, layouts: { _, _ in Self.apart })
                .first { $0.column == .conversation }?.group.terminals == ["conductor"])

        // Show Changes' pane is the orchestrator's own.
        var changes = fleet
        changes.worktrees[0].terminals[1].paneMode = "changes"
        let withChanges = WorkspaceScreen.pane("conductor", host: "", in: changes)!
        #expect(WorkspaceScreen.sharers(of: withChanges, layouts: Self.shared).isEmpty)

        // Meanwhile sleepnomore, opened from the main checkout, isn't drawn
        // a second time there: it's in the column.
        let sleep = ContentView.Selection.workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: "s1"))
        #expect(WorkspaceScreen.shown(sleep, in: fleet, layouts: { _, _ in Self.shared }).map(\.column) == [.conversation])
        // Moved out, it's the checkout's own window.
        #expect(WorkspaceScreen.shown(sleep, in: fleet, layouts: { _, _ in Self.apart }).last?.group.terminals == ["s1"])
    }

    /// A pane somebody split beside the orchestrator, or beside a pane in
    /// its window, was put there on purpose, so it isn't offered a move
    /// (ov-73). One whose origin is unknown (a record from before splits
    /// were recorded, or an older runner), one opened on its own and joined
    /// in later, or one split from a pane that's since gone, still is.
    @Test("Only a pane that wasn't split into the orchestrator's window is offered a move")
    func onlyAPaneThatWasntSplitIntoTheOrchestratorsWindowIsOfferedAMove() {
        let three = [
            PaneGroup(
                id: "@1", name: "", active: true, columns: 122, rows: 24, layout: "@1",
                panes: [Self.pane("conductor", left: 0), Self.pane("s1", left: 41), Self.pane("s2", left: 82)])
        ]
        func sharers(s1: String?, s2: String? = nil) -> [String] {
            var fleet = Self.fleet()
            fleet.worktrees[0].terminals[1].splitOf = s1
            var second = Terminal(id: "s2", short: "s2", title: "logs", preset: "zsh", state: "running", epoch: 0)
            second.splitOf = s2
            fleet.worktrees[0].terminals.append(second)
            let seat = WorkspaceScreen.pane("conductor", host: "", in: fleet)!
            return WorkspaceScreen.sharers(of: seat, layouts: three).map(\.id)
        }

        // Unknown, as every record was before ov-73: today's notice.
        #expect(sharers(s1: nil) == ["s1", "s2"])
        // Split from the orchestrator, or from a pane split from it.
        #expect(sharers(s1: "conductor") == ["s2"])
        #expect(sharers(s1: "conductor", s2: "s1") == [])
        // Split from something outside the window, or since removed.
        #expect(sharers(s1: "elsewhere", s2: "s1") == ["s1"])
    }

    /// The main checkout, opened, knows which orchestrators run in it, so it
    /// can leave them out and say where they are.
    @Test("The main checkout names the orchestrators it hosts")
    func theMainCheckoutNamesTheOrchestratorsItHosts() {
        let fleet = Self.fleet()
        let checkout = fleet.worktrees[0]
        #expect(WorkspaceScreen.seated(in: checkout, fleet: fleet).map(\.workspace.name) == ["Main"])
        #expect(WorkspaceScreen.seated(in: checkout, fleet: fleet).map(\.pane.terminal.id) == ["conductor"])
        // Its own terminals, for the card view, without the orchestrator,
        // or anything in the orchestrator's window.
        #expect(WorkspaceScreen.ownTerminals(of: checkout, fleet: fleet, layouts: Self.apart).terminals.map(\.id) == ["s1"])
        #expect(WorkspaceScreen.ownTerminals(of: checkout, fleet: fleet, layouts: Self.shared).terminals.isEmpty)

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
        #expect(WorkspaceScreen.ownTerminals(of: unseated.worktrees[0], fleet: unseated, layouts: Self.shared).terminals.count == 2)
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
