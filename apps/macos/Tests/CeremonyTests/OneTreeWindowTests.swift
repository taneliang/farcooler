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
        #expect(board.first { $0.action == .boardView }?.title == "Show Tasks by Status")
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

    // MARK: One Needs You count (review H3)

    /// A board read through the stubbed CLI's `task list --json`: two cards
    /// in Needs Decision, so the column and the list can disagree.
    @MainActor
    static func board() async -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let now = Int64(Date().timeIntervalSince1970 * 1000) - 60_000
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            let rows = [("t1", "ov-1", "needs_decision"), ("t2", "ov-2", "needs_decision"), ("t3", "ov-3", "in_progress")]
                .map { id, key, status in
                    #"{"id":"\#(id)","key":"\#(key)","title":"\#(key)","status":"\#(status)","status_since":\#(now),"created_at":\#(now),"updated_at":\#(now)}"#
                }
            return (Data(#"{"tasks":[\#(rows.joined(separator: ","))]}"#.utf8), nil)
        }
        let store = TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
        await store.readIfNeverRead()
        return store
    }

    @MainActor
    @Test("The title bar and the sidebar say one Needs You number, list read or not, served or not")
    func oneCountOnBothSurfaces() async {
        let store = await Self.board()
        #expect(store.board.waitingOnYou == 2)
        for (read, served) in [(true, true), (false, true), (true, false), (false, false)] {
            let count = { (column: Int) in
                WorkspaceNeedsYou.count(items: 1, columnCount: column, listRead: read, listServed: served, themeAsks: 1)
            }
            let title = TitleStatus.model(
                TitleStatusSource(orchestrator: .idle, status: nil, nowDoing: nil, board: store, waiting: count),
                board: store.board)
            let sidebar = OneTree.build(
                OneTreeInput(
                    tasks: store.board.rows.map(OneTreeTask.init(row:)), needsYouCount: count(store.board.waitingOnYou)))
            #expect(sidebar.places[1].detail == "\(title.needYou)", "read \(read), served \(served)")
        }
    }

    @Test("Both surfaces take their count from the one function")
    func bothCallTheOneFunction() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<3 { root.deleteLastPathComponent() }
        let sources = root.appendingPathComponent("Sources/FarCooler")
        let toolbar = try String(contentsOf: sources.appendingPathComponent("ContentView+Toolbar.swift"), encoding: .utf8)
        let tree = try String(contentsOf: sources.appendingPathComponent("Plan/ContentView+OneTree.swift"), encoding: .utf8)
        #expect(toolbar.contains("waiting: { column in needsYouCount(host: host, workspace: summary, client: client, columnCount: column) }"))
        #expect(tree.contains("needsYouCount: needsYouCount(\n                host: host, workspace: workspace, client: client, columnCount: board.board.waitingOnYou)"))
        #expect(tree.contains("WorkspaceNeedsYou.count("))
    }
}
