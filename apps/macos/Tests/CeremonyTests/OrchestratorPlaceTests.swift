import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A terminal is shown in exactly one place (ov-63): an orchestrator in its
/// workspace's conversation column, and nowhere else, however many other
/// terminals share its worktree or its tmux window. And the orchestrator is
/// one pane (ov-78): nothing is split into its window or joins it.
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

    /// The owner's report (ov-78): a pane in the orchestrator's window, a
    /// split made in its column before ov-78 among them, is listed under the
    /// main checkout but couldn't be opened there, because the checkout left
    /// out everything the column draws. Now it's named in the column, with
    /// Move to Its Own Window, and opening it from the checkout says which
    /// seat's window it has to leave first. Nothing is moved unasked.
    @Test("A pane sharing the orchestrator's window is named, and opening it from the checkout moves it")
    func aPaneSharingTheOrchestratorsWindowIsNamedAndOpeningItMovesIt() {
        let fleet = Self.fleet()
        let shown = WorkspaceScreen.shown(Self.workspace, in: fleet, layouts: { _, _ in Self.shared })
        // Until it's moved the column draws the window whole: tmux can't
        // draw one pane of a window apart from the rest.
        #expect(shown.first { $0.column == .conversation }?.group.terminals == ["conductor", "s1"])
        let seat = WorkspaceScreen.pane("conductor", host: "", in: fleet)!
        #expect(WorkspaceScreen.sharers(of: seat, layouts: Self.shared).map(\.id) == ["s1"])
        #expect(SharedWindowNotice.sentence("sleepnomore") == "sleepnomore shares the orchestrator’s window.")
        let checkout = fleet.worktrees[0]
        #expect(WorkspaceScreen.seat(sharedBy: "s1", in: checkout, fleet: fleet, layouts: Self.shared)?.terminal.id == "conductor")
        #expect(WorkspaceScreen.seat(sharedBy: "conductor", in: checkout, fleet: fleet, layouts: Self.shared) == nil)

        // Apart, or not read yet: nothing to say or move.
        #expect(WorkspaceScreen.sharers(of: seat, layouts: Self.apart).isEmpty)
        #expect(WorkspaceScreen.sharers(of: seat, layouts: nil).isEmpty)
        #expect(WorkspaceScreen.seat(sharedBy: "s1", in: checkout, fleet: fleet, layouts: Self.apart) == nil)
        #expect(
            WorkspaceScreen.shown(Self.workspace, in: fleet, layouts: { _, _ in Self.apart })
                .first { $0.column == .conversation }?.group.terminals == ["conductor"])

        // A changes pane in the window is a sharer too: Show Changes no
        // longer opens one there.
        var changes = fleet
        changes.worktrees[0].terminals[1].paneMode = "changes"
        let withChanges = WorkspaceScreen.pane("conductor", host: "", in: changes)!
        #expect(WorkspaceScreen.sharers(of: withChanges, layouts: Self.shared).map(\.id) == ["s1"])

        // Opened from the main checkout while it's shared, it isn't drawn a
        // second time there; moved out, it's the checkout's own window.
        let sleep = ContentView.Selection.workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: "s1"))
        #expect(WorkspaceScreen.shown(sleep, in: fleet, layouts: { _, _ in Self.shared }).map(\.column) == [.conversation])
        #expect(WorkspaceScreen.shown(sleep, in: fleet, layouts: { _, _ in Self.apart }).last?.group.terminals == ["s1"])
    }

    /// Every pane in the orchestrator's window is offered the move, however
    /// it got there: ov-73 and ov-76 spared a split made in the column, and
    /// that split is exactly the terminal the owner couldn't open. Another
    /// orchestrator is never moved on this one's behalf.
    @Test("Every pane in the orchestrator's window but an orchestrator is offered a move")
    func everyPaneInTheOrchestratorsWindowIsOfferedAMove() {
        let three = [
            PaneGroup(
                id: "@1", name: "", active: true, columns: 122, rows: 24, layout: "@1",
                panes: [Self.pane("conductor", left: 0), Self.pane("s1", left: 41), Self.pane("s2", left: 82)])
        ]
        var fleet = Self.fleet()
        fleet.worktrees[0].terminals.append(
            Terminal(id: "s2", short: "s2", title: "logs", preset: "zsh", state: "running", epoch: 0))
        let seat = WorkspaceScreen.pane("conductor", host: "", in: fleet)!
        #expect(WorkspaceScreen.sharers(of: seat, layouts: three).map(\.id) == ["s1", "s2"])

        fleet.worktrees[0].terminals[2].role = "orchestrator"
        #expect(WorkspaceScreen.sharers(of: WorkspaceScreen.pane("conductor", host: "", in: fleet)!, layouts: three).map(\.id) == ["s1"])
    }

    /// The orchestrator is one pane (ov-78). ⌃B %, ⌃B " and ⌃B c with the
    /// keyboard in its column open a shell in the main checkout instead;
    /// with the keyboard anywhere else they split as before. And a drop
    /// never moves the orchestrator or puts a pane in its window.
    @Test("Nothing splits into the orchestrator's window or joins it")
    func nothingSplitsIntoTheOrchestratorsWindowOrJoinsIt() {
        let fleet = Self.fleet()
        let shown = WorkspaceScreen.shown(Self.workspace, in: fleet, layouts: { _, _ in Self.apart })
        let conductor = PaneRef(host: "", worktree: "checkout", terminal: "conductor")
        for command in [TileCommand.splitRight, .splitDown, .newGroup] {
            #expect(WorkspaceScreen.opensShellInstead(command, key: conductor, in: shown))
        }
        #expect(!WorkspaceScreen.opensShellInstead(.zoom, key: conductor, in: shown))
        #expect(!WorkspaceScreen.opensShellInstead(.splitRight, key: nil, in: shown))
        // In the checkout's own window, opened beside it, a split is a split.
        let opened = WorkspaceScreen.shown(
            .workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: "s1")), in: fleet,
            layouts: { _, _ in Self.apart })
        let s1 = PaneRef(host: "", worktree: "checkout", terminal: "s1")
        #expect(!WorkspaceScreen.opensShellInstead(.splitRight, key: s1, in: opened))

        let checkout = fleet.worktrees[0]
        #expect(WorkspaceScreen.joinsOrchestrator("s1", window: ["conductor"], in: checkout, fleet: fleet))
        #expect(WorkspaceScreen.joinsOrchestrator("conductor", window: ["s1"], in: checkout, fleet: fleet))
        #expect(WorkspaceScreen.joinsOrchestrator("s2", window: ["conductor", "s1"], in: checkout, fleet: fleet))
        #expect(!WorkspaceScreen.joinsOrchestrator("s2", window: ["s1"], in: checkout, fleet: fleet))
    }

    /// Show Changes is reachable with no terminal open (ov-78): the
    /// toolbar's Changes acts on a worktree opened whole, whatever it holds,
    /// and beside the Orchestrator column on the main checkout, never the
    /// orchestrator's window. Not for a task, whose column has its changes.
    @Test("The toolbar's Changes names a worktree with no terminal, and the checkout beside the orchestrator")
    func theToolbarsChangesNamesAWorktreeWithNoTerminal() {
        var fleet = Self.fleet()
        fleet.worktrees.append(
            Worktree(
                id: "lane", short: "ln", task: "lane", branch: "lane", repository: "overnight", host: "",
                path: "/tmp/ln", state: "active", terminals: [], repositoryID: Self.repo, workspace: Self.main))
        let lane = ContentView.Selection.workspace(host: "", workspace: Self.main, focus: .worktree("lane", terminal: nil))
        #expect(WorkspaceScreen.changesTarget(lane, in: fleet)?.id == "lane")
        #expect(WorkspaceScreen.changesTarget(.looseWorktree(host: "", worktree: "lane", terminal: nil), in: fleet)?.id == "lane")
        #expect(WorkspaceScreen.changesTarget(Self.workspace, in: fleet)?.id == "checkout")
        #expect(WorkspaceScreen.changesTarget(.workspace(host: "", workspace: Self.main, focus: .task("t-1")), in: fleet) == nil)
        #expect(WorkspaceScreen.changesTarget(.needsYou, in: fleet) == nil)
        var none = fleet
        none.runnerWorkspaces[""] = [
            WorkspaceSummary(id: Self.main, name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: Self.repo)
        ]
        none.worktrees[0].terminals[0].state = "exited"
        #expect(WorkspaceScreen.changesTarget(Self.workspace, in: none) == nil)
    }

    /// The main checkout, opened, knows which orchestrators run in it, so it
    /// can leave them out and say where they are.
    @Test("The main checkout names the orchestrators it hosts")
    func theMainCheckoutNamesTheOrchestratorsItHosts() {
        let fleet = Self.fleet()
        let checkout = fleet.worktrees[0]
        #expect(WorkspaceScreen.seated(in: checkout, fleet: fleet).map(\.workspace.name) == ["Main"])
        #expect(WorkspaceScreen.seated(in: checkout, fleet: fleet).map(\.pane.terminal.id) == ["conductor"])
        // Its own terminals, for the card view: all but the orchestrator,
        // a pane sharing its window included, which opening moves out.
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
