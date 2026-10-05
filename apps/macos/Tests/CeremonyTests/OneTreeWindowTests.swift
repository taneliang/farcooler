import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The window's side of the one tree (ov-321): which node a selection is,
/// Needs You as a canvas page kept across a relaunch, and the Board view
/// reached from ⌘K. The tree's own rules are AgentKit's (`OneTreeTests`).
struct OneTreeWindowTests {
    typealias Selection = ContentView.Selection

    @Test("Each place a workspace can show is the node it lights in the tree")
    func selectionsAreNodes() {
        let at = { (focus: ContentView.Focus?) in
            ContentView.treeTarget(Selection.workspace(host: "", workspace: "w", focus: focus), workspace: "w")
        }
        #expect(at(nil) == .plan)
        #expect(at(.task("t")) == .task("t"))
        #expect(at(.plan(.theme("th"))) == .theme("th"))
        #expect(at(.plan(.lane("l"))) == .lane("l"))
        #expect(at(.plan(.page("p"))) == .page("p"))
        #expect(at(.plan(.needsYou)) == .needsYou)
        #expect(at(.worktree("wt", terminal: "x")) == .terminal(worktree: "wt", terminal: "x"))
        #expect(at(.worktree("wt", terminal: nil)) == .worktree("wt"))
        #expect(at(.history(.done)) == nil)
        // Another workspace's place is no node of this tree.
        #expect(ContentView.treeTarget(.workspace(host: "", workspace: "other", focus: nil), workspace: "w") == nil)
        #expect(
            ContentView.treeTarget(.looseWorktree(host: "", worktree: "wt", terminal: "x"), workspace: "w")
                == .terminal(worktree: "wt", terminal: "x"))
    }

    @Test("Needs You on the canvas is kept across a relaunch like any plan page")
    func needsYouIsKept() {
        let place = Selection.workspace(host: "h", workspace: "w", focus: .plan(.needsYou))
        let saved = SelectionMemory.encode(place)
        #expect(saved == "h|w|plan:needs-you")
        #expect(saved.flatMap(SelectionMemory.decode) == place)
    }

    @Test("⌘K offers the Board view in a workspace, and the way back to the tree")
    func boardViewInThePalette() {
        let board = PaletteIndex.matching("board", in: [], boardView: false)
        #expect(board.first { $0.action == .boardView }?.title == "Show Board")
        let tree = PaletteIndex.matching("board", in: [], boardView: true)
        #expect(tree.first { $0.action == .boardView }?.title == "Show Tree")
        #expect(!PaletteIndex.matching("board", in: [], boardView: nil).contains { $0.action == .boardView })
    }

    @Test("A worktree's panes as the tree reads them: the label, agent or shell, and the orchestrator kept apart")
    func terminalsAsTheTreeReadsThem() {
        let shell = Terminal(id: "a", short: "a", title: "Terminal 3", preset: "zsh", state: "running", epoch: 0)
        var agent = Terminal(id: "b", short: "b", title: "fixing totals", preset: "claude", state: "running", epoch: 0)
        agent.activity = "working"
        #expect(ContentView.treeTerminal(shell) == OneTreeTerminal(id: "a", title: shell.label, isAgent: false))
        #expect(ContentView.treeTerminal(agent) == OneTreeTerminal(id: "b", title: "fixing totals", isAgent: true))
    }
}
