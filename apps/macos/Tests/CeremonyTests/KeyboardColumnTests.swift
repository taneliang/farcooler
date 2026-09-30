import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Which column the keyboard is in, with two terminal views on screen.
@MainActor
struct KeyboardColumnTests {
    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let main = "0198f2c0-0000-7000-8000-0000000000cc"

    /// Main's orchestrator runs in the main checkout, which Main's
    /// Worktrees can open whole beside it.
    private static func fleet() -> Fleet {
        var conductor = Terminal(id: "conductor", short: "cd", title: "orchestrator", preset: "claude", state: "running", epoch: 0)
        conductor.role = "orchestrator"
        conductor.workspace = main
        let shell = Terminal(id: "s1", short: "s1", title: "zsh", preset: "zsh", state: "running", epoch: 0)
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

    private static func layouts() -> [PaneGroup] {
        func group(_ id: String, _ pane: String, active: Bool) -> PaneGroup {
            PaneGroup(
                id: id, name: "", active: active, columns: 80, rows: 24, layout: id,
                panes: [PaneRect(id: pane, short: pane, title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: true, zoomed: false)])
        }
        return [group("@1", "s1", active: false), group("@2", "conductor", active: true)]
    }

    /// Clicking, or stepping with ⌘], into the conversation while the main
    /// checkout is open whole leaves the selection alone: naming the
    /// orchestrator as the checkout's pane drew its window in the third
    /// column as well. Even named, it's never the worktree column's.
    @Test("The conversation's pane never lands in the opened main checkout")
    func theConversationsPaneNeverLandsInTheOpenedMainCheckout() {
        let fleet = Self.fleet()
        let opened = ContentView.Selection.workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: nil))
        let shown = WorkspaceScreen.shown(opened, in: fleet, layouts: { _, _ in Self.layouts() })
        #expect(shown.map(\.column) == [.conversation, .worktree])
        #expect(shown.last?.group.id == "@1")
        let conductor = PaneRef(host: "", worktree: "checkout", terminal: "conductor")
        #expect(WorkspaceScreen.focusing(conductor, selection: opened, shown: shown, fleet: fleet) == .some(opened))
        // Its shell names itself.
        let shell = PaneRef(host: "", worktree: "checkout", terminal: "s1")
        #expect(
            WorkspaceScreen.focusing(shell, selection: opened, shown: shown, fleet: fleet)
                == .some(.workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: "s1"))))
        // Named anyway, the worktree column still shows the checkout's own.
        let named = ContentView.Selection.workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: "conductor"))
        #expect(WorkspaceScreen.shown(named, in: fleet, layouts: { _, _ in Self.layouts() }).last?.group.id == "@1")
    }

    /// Only the view holding the key pane has the keyboard, so typed keys,
    /// ⌃B and ⌘W reach one pane; after ⌥⌘2 neither does.
    @Test("Only the column holding the key pane takes typed keys")
    func onlyTheColumnHoldingTheKeyPaneTakesTypedKeys() {
        let fleet = Self.fleet()
        let opened = ContentView.Selection.workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: nil))
        let shown = WorkspaceScreen.shown(opened, in: fleet, layouts: { _, _ in Self.layouts() })
        let conductor = PaneRef(host: "", worktree: "checkout", terminal: "conductor")
        #expect(shown.map { WorkspaceScreen.hasKeyboard($0, key: conductor, onBoard: false) } == [true, false])
        let shell = PaneRef(host: "", worktree: "checkout", terminal: "s1")
        #expect(shown.map { WorkspaceScreen.hasKeyboard($0, key: shell, onBoard: false) } == [false, true])
        #expect(shown.map { WorkspaceScreen.hasKeyboard($0, key: shell, onBoard: true) } == [false, false])
        #expect(shown.map { WorkspaceScreen.hasKeyboard($0, key: nil, onBoard: false) } == [false, false])
    }

    /// While it starts, the column draws "Starting Orchestrator…", not its
    /// pane, so the pane isn't on screen: not marked seen, not watched.
    @Test("A starting orchestrator isn't on screen")
    func aStartingOrchestratorIsntOnScreen() {
        var fleet = Self.fleet()
        let workspace = ContentView.Selection.workspace(host: "", workspace: Self.main, focus: nil)
        #expect(WorkspaceScreen.shown(workspace, in: fleet, layouts: { _, _ in Self.layouts() }).map(\.column) == [.conversation])
        fleet.worktrees[0].terminals[0].state = "starting"
        #expect(WorkspaceScreen.shown(workspace, in: fleet, layouts: { _, _ in Self.layouts() }).isEmpty)
    }

    /// A workspace's row stays lit with a task open in it; with one of its
    /// worktrees open, that worktree's row is lit instead.
    @Test("A workspace row stays lit with a task open")
    func aWorkspaceRowStaysLitWithATaskOpen() {
        // Bound first: inside `#expect`, a bare `.task(…)` doesn't reach
        // `Focus.task`, and the expectation passed whatever the rule said.
        let task: ContentView.Focus = .task("t-9")
        let worktree: ContentView.Focus = .worktree("w", terminal: nil)
        let lit = ContentView.highlightsWorkspace(nil)
        let withTask = ContentView.highlightsWorkspace(task)
        let withWorktree = ContentView.highlightsWorkspace(worktree)
        #expect(lit)
        #expect(withTask)
        #expect(!withWorktree)
    }
}
