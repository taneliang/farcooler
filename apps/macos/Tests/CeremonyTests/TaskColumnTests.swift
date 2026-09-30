import AppKit
import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A workspace's task column (spec §4.4).
@MainActor
struct TaskColumnTests {
    private static func row(_ status: TaskStatus, worktree: String? = nil) -> TaskRow {
        var row = TaskRow(id: "t-9", key: "bil-9", title: "Invoice PDF export", status: status, statusSince: .now)
        row.worktreeID = worktree
        return row
    }

    /// In Review, a task is its changes: they take the larger share. Any
    /// other status leads with the agent. A divider the person dragged wins,
    /// kept apart for the two kinds, and never past the edge.
    @Test("A task in review leads with its changes")
    func aTaskInReviewLeadsWithItsChanges() {
        #expect(TaskColumnModel.agentShare(status: .inReview, stored: nil) < 0.5)
        #expect(TaskColumnModel.agentShare(status: .inProgress, stored: nil) > 0.5)
        #expect(TaskColumnModel.agentShare(status: .needsDecision, stored: nil) > 0.5)
        #expect(TaskColumnModel.agentShare(status: .inReview, stored: 0.6) == 0.6)
        #expect(TaskColumnModel.agentShare(status: .inReview, stored: 0.99) == 1 - TaskColumnModel.minimumShare)
        // And the card starts open where its question is.
        #expect(TaskColumnModel.startsExpanded(.needsDecision))
        #expect(!TaskColumnModel.startsExpanded(.inReview))
    }

    /// A task in review with no live agent can still reach its changes: its
    /// worktree, from `Task.worktree_id`.
    @Test("A task with no agent but a worktree offers Open Worktree")
    func aTaskWithNoAgentButAWorktreeOffersOpenWorktree() {
        let row = Self.row(.inReview, worktree: "w-3")
        let lane = TaskColumnModel.worktree(of: row, agent: nil)
        #expect(lane == "w-3")
        let agent = TaskColumnModel.agent(hasAgent: false, worktree: lane)
        #expect(agent == .none(openWorktree: true))
        #expect(TaskColumnModel.sentence(agent) == "No agent is working on this task.")
        #expect(TaskColumnModel.sentence(TaskColumnModel.agent(hasAgent: true, worktree: lane)) == nil)
    }

    @Test("A task with neither says nothing has started")
    func aTaskWithNeitherSaysNothingHasStarted() {
        let row = Self.row(.todo)
        let agent = TaskColumnModel.agent(hasAgent: false, worktree: TaskColumnModel.worktree(of: row, agent: nil))
        #expect(agent == .none(openWorktree: false))
        #expect(TaskColumnModel.sentence(agent) == "Nothing has started on this task yet.")
        // An empty id from the runner is no worktree either.
        #expect(TaskColumnModel.worktree(of: Self.row(.todo, worktree: ""), agent: nil) == nil)
    }

    /// The task column draws the worktree's changes from its store, with no
    /// tmux pane, so nothing resizes the agent's window for other clients
    /// (ruling 6). The toolbar's Changes button in an opened worktree still
    /// splits a pane, and the recorder sees it, which is what shows it could
    /// see one.
    @Test("Opening a task's changes runs no split, and the toolbar's Changes button does")
    func openingATasksChangesRunsNoSplit() async {
        let worktree = Worktree(
            id: "w-3", short: "w3", task: "fc-3-webhooks", branch: "b", repository: "overnight", host: "",
            path: "/tmp/w3", state: "active", terminals: [])
        let column = WorktreeCallsTests.Recorder()
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in column.answer(args) }
        let store = ChangesStore(client: client, worktree: worktree)
        await store.load()
        #expect(!column.calls.isEmpty, "the changes were never read")
        #expect(!column.calls.contains { $0.starts(with: ["layout", "split"]) }, "\(column.calls)")

        let toolbar = WorktreeCallsTests.Recorder()
        let other = DaemonClient(target: "", notifications: NotificationCenter())
        other.commandRunnerForTesting = { args in toolbar.answer(args) }
        _ = await other.split(worktree, beside: nil, side: .right, preset: "changes", layout: nil)
        #expect(toolbar.calls.contains { $0.starts(with: ["layout", "split"]) && $0.contains("changes") }, "\(toolbar.calls)")
    }

    /// Open Worktree puts the worktree whole in the third column, its own
    /// layouts in the bar, and Back returns to the task it came from. From
    /// the Worktrees disclosure, with no task behind it, Back closes the
    /// column.
    @Test("Open Worktree shows the worktree's own layouts, and Back returns to the task")
    func openWorktreeShowsTheWorktreesOwnLayoutsAndBackReturnsToTheTask() {
        let task = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .task("t-9"))
        let opened = WorkspaceNavigation.openWorktree("w-3", from: task)
        #expect(opened.next == .workspace(host: "", workspace: "ws", focus: .worktree("w-3", terminal: nil)))
        #expect(opened.trail == task)

        var shell = Terminal(id: "s", short: "s", title: "zsh", preset: "zsh", state: "running", epoch: 0)
        shell.taskId = nil
        let lane = Worktree(
            id: "w-3", short: "w3", task: "fc-3-webhooks", branch: "b", repository: "overnight", host: "",
            path: "/tmp/w3", state: "active", terminals: [shell])
        func group(_ id: String, active: Bool) -> PaneGroup {
            PaneGroup(
                id: id, name: "", active: active, columns: 80, rows: 24, layout: id,
                panes: [PaneRect(id: "s", short: "s", title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: true, zoomed: false)])
        }
        let fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [lane], branchPrefix: nil)
        let shown = WorkspaceScreen.shown(opened.next, in: fleet, layouts: { _, _ in [group("@1", active: true), group("@2", active: false)] })
        #expect(shown.map(\.column) == [.worktree])
        #expect(shown.first?.groups.map(\.id) == ["@1", "@2"], "not the worktree's own layouts")

        #expect(WorkspaceNavigation.back(from: opened.next, trail: opened.trail) == task)
        #expect(WorkspaceNavigation.back(from: opened.next, trail: nil) == .workspace(host: "", workspace: "ws", focus: nil))
        #expect(WorkspaceNavigation.back(from: task, trail: nil) == .workspace(host: "", workspace: "ws", focus: nil))
        #expect(WorkspaceNavigation.back(from: .workspace(host: "", workspace: "ws", focus: nil), trail: task) == nil)
    }

    /// Esc goes Back only with something to go back from, and never while a
    /// terminal or a text field has the keyboard: a terminal needs its Esc,
    /// and a field cancels with it.
    @Test("Esc goes back only when no terminal or field has the keyboard")
    func escGoesBackOnlyWhenNoTerminalOrFieldHasTheKeyboard() {
        let task = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .task("t-9"))
        let plain = NSView()
        #expect(EscapeBack.goesBack(responder: plain, selection: task, focusColumn: false))
        #expect(EscapeBack.goesBack(responder: nil, selection: task, focusColumn: false))
        #expect(!EscapeBack.goesBack(responder: TerminalRenderView(), selection: task, focusColumn: false))
        #expect(!EscapeBack.goesBack(responder: NSTextView(), selection: task, focusColumn: false))
        #expect(!EscapeBack.goesBack(responder: NSTextField(), selection: task, focusColumn: false))
        // Nothing to go back from.
        let workspace = ContentView.Selection.workspace(host: "", workspace: "ws", focus: nil)
        #expect(!EscapeBack.goesBack(responder: plain, selection: workspace, focusColumn: false))
        #expect(!EscapeBack.goesBack(responder: plain, selection: .needsYou, focusColumn: false))
        #expect(EscapeBack.goesBack(responder: plain, selection: task, focusColumn: true))
    }
}
