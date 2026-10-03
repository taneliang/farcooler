import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The workspace navigator (ov-92): what's selected in it and what ↑, ↓ and
/// Esc do to that; which rows each section holds; and what the
/// orchestrator's row says it's doing, from what the runner already sends.
@MainActor
struct NavigatorTests {
    private typealias Selection = ContentView.Selection

    private static let board = "ws-bil"
    private static let orchestrator = Selection.workspace(host: "", workspace: board, focus: nil)
    private static func task(_ id: String) -> Selection { .workspace(host: "", workspace: board, focus: .task(id)) }
    private static func worktree(_ id: String) -> Selection {
        .workspace(host: "", workspace: board, focus: .worktree(id, terminal: nil))
    }

    // MARK: - The selection

    /// The workspace's own level is the orchestrator selected: it's the
    /// default, what a workspace opens on, and what the row lights. A task
    /// lights its row, as does a worktree opened from it; a loose worktree
    /// lights its own. Another board's selection lights nothing here.
    @Test("The orchestrator is selected by default; a task or a worktree lights its own row")
    func theOrchestratorIsTheDefault() {
        func current(_ selection: Selection?, trail: Selection? = nil) -> NavigatorItem? {
            Navigator.current(selection, trail: trail, board: Self.board)
        }
        #expect(current(Self.orchestrator) == .orchestrator)
        #expect(current(Self.task("t3")) == .task("t3"))
        #expect(current(Self.worktree("scratch")) == .worktree("scratch"))
        #expect(current(.looseWorktree(host: "", worktree: "stray", terminal: "t")) == .worktree("stray"))
        // Open Worktree from a task: the task stays lit.
        let lane = WorkspaceNavigation.openWorktree("tax", from: Self.task("t3"))
        #expect(current(lane.next, trail: lane.trail) == .task("t3"))
        #expect(current(.workspace(host: "", workspace: "ws-other", focus: nil)) == nil)
        #expect(current(.needsYou) == nil)
        #expect(current(nil) == nil)
    }

    /// Esc goes back to the orchestrator, from a task or a worktree, past
    /// the task a worktree was opened from; ⌃⌘← goes along the breadcrumb.
    /// Focus is left first, one press each. At the orchestrator, there's
    /// nowhere to go.
    @Test("Esc goes back to the orchestrator; ⌃⌘← along the breadcrumb")
    func escGoesToTheOrchestrator() {
        typealias Step = WorkspaceNavigation.BackStep
        let lane = WorkspaceNavigation.openWorktree("tax", from: Self.task("t3"))
        func esc(_ from: Selection?, trail: Selection? = nil, focus: Bool = false) -> Step {
            WorkspaceNavigation.backStep(focus: focus, oneAtATime: true, toOrchestrator: true, from: from, trail: trail)
        }
        #expect(esc(Self.task("t3")) == Step(goesTo: Self.orchestrator))
        #expect(esc(Self.worktree("scratch")) == Step(goesTo: Self.orchestrator))
        #expect(esc(lane.next, trail: lane.trail) == Step(goesTo: Self.orchestrator))
        #expect(esc(Self.task("t3"), focus: true) == Step(leavesFocus: true))
        #expect(esc(Self.orchestrator) == Step(goesTo: nil))
        // ⌃⌘←: from the worktree to the task it was opened from.
        let back = WorkspaceNavigation.backStep(focus: false, oneAtATime: true, from: lane.next, trail: lane.trail)
        #expect(back == Step(goesTo: Self.task("t3")))
        // And Esc only goes back with something to go back from, and never
        // from a terminal, which keeps its Esc.
        #expect(!EscapeBack.goesBack(responder: nil, selection: Self.orchestrator, focusColumn: false))
        #expect(EscapeBack.goesBack(responder: nil, selection: Self.task("t3"), focusColumn: false))
        #expect(!EscapeBack.goesBack(responder: TerminalRenderView(), selection: Self.task("t3"), focusColumn: false))
    }

    /// ↑ and ↓ walk the whole navigator, top to bottom: the orchestrator,
    /// the tasks as the list shows them, then the loose worktrees, held at
    /// either end. From a row not in the list (a task's worktree opened
    /// whole), the first going down and the last going up.
    @Test("↑ and ↓ walk the orchestrator, the tasks, then the worktrees")
    func arrowsWalkEverySection() {
        let items = Navigator.items(orchestrator: true, tasks: ["t3", "t9"], worktrees: ["scratch"])
        #expect(items == [.orchestrator, .task("t3"), .task("t9"), .worktree("scratch")])
        var at: NavigatorItem? = .orchestrator
        var walked: [NavigatorItem] = []
        for _ in 0..<5 {
            at = Navigator.step(from: at, by: 1, in: items)
            walked.append(at!)
        }
        #expect(walked == [.task("t3"), .task("t9"), .worktree("scratch"), .worktree("scratch"), .worktree("scratch")])
        #expect(Navigator.step(from: .task("t3"), by: -1, in: items) == .orchestrator)
        #expect(Navigator.step(from: .orchestrator, by: -1, in: items) == .orchestrator)
        #expect(Navigator.step(from: .worktree("tax"), by: 1, in: items) == .orchestrator)
        #expect(Navigator.step(from: nil, by: -1, in: items) == .worktree("scratch"))
        #expect(Navigator.step(from: nil, by: 1, in: []) == nil)
        // No orchestrator (a runner without workspaces): the tasks lead.
        #expect(Navigator.items(orchestrator: false, tasks: ["t3"], worktrees: []) == [.task("t3")])
    }

    // MARK: - The orchestrator's row

    private static func seat(
        activity: String?, state: String = "running", line: String? = nil, said: String? = nil,
        question: String? = nil, since: Double? = nil
    ) -> BoardPane {
        var t = Terminal(id: "conductor", short: "c", title: "claude", preset: "claude", state: state, epoch: 0)
        t.activity = activity
        t.line = line
        t.said = said
        t.blockedQuestion = question
        t.activitySince = since
        t.role = "orchestrator"
        let worktree = Worktree(
            id: "checkout", short: "c", task: "main", branch: "main", repository: "shop", host: "", path: "/tmp/c",
            state: "active", terminals: [t])
        return BoardPane(terminal: t, worktree: worktree)
    }

    /// Its state is its own: working, asking, finished unseen, idle or
    /// stopped, by its pane; never the workspace's tasks waiting on you,
    /// which have rows of their own.
    @Test("The row's state comes from the orchestrator's own pane")
    func theRowsStateIsItsOwn() {
        #expect(OrchestratorRow.state(seat: nil) == .none)
        #expect(OrchestratorRow.state(seat: Self.seat(activity: "working")) == .working)
        #expect(OrchestratorRow.state(seat: Self.seat(activity: "blocked")) == .needsYou)
        #expect(OrchestratorRow.state(seat: Self.seat(activity: "done")) == .unread)
        #expect(OrchestratorRow.state(seat: Self.seat(activity: "idle")) == .idle)
        #expect(OrchestratorRow.state(seat: Self.seat(activity: nil, state: "starting")) == .starting)
        #expect(OrchestratorRow.state(seat: Self.seat(activity: nil, state: "lost")) == .stopped)
        #expect(
            [OrchestratorRow.State.working, .idle, .needsYou, .stopped].map(OrchestratorRow.word)
                == ["Working", "Idle", "Needs You", "Stopped"])
        #expect(OrchestratorRow.word(.none) == "No Orchestrator")
        #expect(OrchestratorRow.accessibilityLabel(agent: "claude", state: .working) == "Orchestrator, claude, Working")
    }

    /// What it's doing now, from what its pane already carries: the question
    /// it's blocked on; working, its hook-reported activity or plan position
    /// (`line`), else what it last said; finished or idle, what it last
    /// said, else "Idle since" its last change. Nothing when stopped,
    /// starting, or with none.
    @Test("The now-doing line comes from the pane's own line, question and last words")
    func theNowDoingLine() {
        func doing(_ seat: BoardPane?) -> String? {
            OrchestratorRow.nowDoing(seat?.terminal, state: OrchestratorRow.state(seat: seat), time: { _ in "3:42 PM" })
        }
        #expect(doing(Self.seat(activity: "working", line: "Running the Mac tests for ov-91", said: "Next")) == "Running the Mac tests for ov-91")
        #expect(doing(Self.seat(activity: "working", line: "  ", said: "Reading the brief")) == "Reading the brief")
        #expect(doing(Self.seat(activity: "blocked", line: "claude needs you", question: "Ship ov-92?")) == "Ship ov-92?")
        #expect(doing(Self.seat(activity: "blocked", line: "claude needs you")) == "claude needs you")
        #expect(doing(Self.seat(activity: "idle", line: "claude idle", said: "All three lanes are merged.")) == "All three lanes are merged.")
        #expect(doing(Self.seat(activity: "done", line: "claude done", said: "Done: ov-90 landed.")) == "Done: ov-90 landed.")
        #expect(doing(Self.seat(activity: "idle", line: "claude idle", since: 1_759_412_520_000)) == "Idle since 3:42 PM")
        #expect(doing(Self.seat(activity: "idle")) == nil)
        #expect(doing(Self.seat(activity: nil, state: "lost", line: "x", said: "y")) == nil)
        #expect(doing(nil) == nil)
        #expect(OrchestratorRow.inProgress(0) == nil)
        #expect(OrchestratorRow.inProgress(1) == "1 task in progress")
        #expect(OrchestratorRow.inProgress(3) == "3 tasks in progress")
    }
}
