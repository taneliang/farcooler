import Foundation
import Testing

@testable import AgentKit

// Which task a pane's header names. Android's `TaskLinkTest.kt` states the same
// cases, so the phones and the Mac agree about one pane.

/// A pane with only what the two rules read.
private struct Pane: TaskLinkPane, TaskBoardPane {
    var boardTaskID: String?
    var boardState: String = "running"
    var runsAgent: Bool = true
    var isOrchestrator: Bool = false
    var workspace: String? = nil
}

private struct Lane: TaskLinkWorktree {
    var openTaskIDs: [String]
    var isRepositoryCheckout = false
}

private let bil9 = "0198f2c0-0000-7000-8000-00000000a009"
private let bil7 = "0198f2c0-0000-7000-8000-00000000a007"

@Test("A dispatched pane's task is its own, over its worktree's")
func aDispatchedPanesTaskIsItsOwnOverItsWorktrees() {
    #expect(TaskLink.task(of: Pane(boardTaskID: bil7), in: Lane(openTaskIDs: [bil9])) == bil7)
    // Even where its own task has left the worktree's open list: Done, or
    // moved to another lane. The pane was opened for it all the same.
    #expect(TaskLink.task(of: Pane(boardTaskID: bil7), in: Lane(openTaskIDs: [])) == bil7)
    #expect(
        TaskLink.task(of: Pane(boardTaskID: bil7), in: Lane(openTaskIDs: [bil9, bil7])) == bil7)
}

@Test("A pane in a worktree with one open task shows that task")
func aPaneInAWorktreeWithOneOpenTaskShowsThatTask() {
    #expect(TaskLink.task(of: Pane(boardTaskID: nil), in: Lane(openTaskIDs: [bil9])) == bil9)
    // An empty id is no id: the runner never sends one, but it isn't a task.
    #expect(TaskLink.task(of: Pane(boardTaskID: ""), in: Lane(openTaskIDs: [bil9])) == bil9)
    #expect(TaskLink.task(of: Pane(boardTaskID: nil), in: Lane(openTaskIDs: [])) == nil)
}

@Test("A pane in a worktree with two open tasks shows none")
func aPaneInAWorktreeWithTwoOpenTasksShowsNone() {
    #expect(TaskLink.task(of: Pane(boardTaskID: nil), in: Lane(openTaskIDs: [bil9, bil7])) == nil)
}

@Test("An orchestrator's pane names no task")
func anOrchestratorsPaneNamesNoTask() {
    // It leads the workspace, even from a worktree with exactly one open task,
    // and even with a task id of its own.
    let lead = Pane(boardTaskID: nil, isOrchestrator: true)
    #expect(TaskLink.task(of: lead, in: Lane(openTaskIDs: [bil9])) == nil)
    let dispatched = Pane(boardTaskID: bil7, isOrchestrator: true)
    #expect(TaskLink.task(of: dispatched, in: Lane(openTaskIDs: [bil9])) == nil)
}

@Test("A shell shown under a task is not counted as that task's agent")
func aShellShownUnderATaskIsNotCountedAsThatTasksAgent() {
    // A person's shell, and an agent somebody started by hand: neither was
    // dispatched for the task, and `task_id` stays explicit.
    let shell = Pane(boardTaskID: nil, runsAgent: false)
    let byHand = Pane(boardTaskID: nil, runsAgent: true)
    let lane = Lane(openTaskIDs: [bil9])
    #expect(TaskLink.task(of: shell, in: lane) == bil9, "its header names the task")
    #expect(TaskLink.task(of: byHand, in: lane) == bil9)

    let board = TaskBoardModel(columns: [
        TaskBoardColumn(
            status: .inProgress,
            rows: [
                TaskRow(
                    id: bil9, key: "bil-9", title: "Invoice PDF export", status: .inProgress,
                    statusSince: .now)
            ])
    ])
    #expect(board.tasksWithLiveAgents(in: [shell, byHand]) == 0, "and neither is its agent")
    #expect(
        board.rows[0].agentPresence(
            livePanes: board.rows[0].livePanes(in: [shell, byHand]).count,
            runnerRecordsTasks: true) == .noAgent)
}

/// The phone's own types answer through the fleet it decodes: `open_tasks`
/// on the worktree and `taskId` on the terminal.
@Test("The phone's fleet names a pane's task from open_tasks")
func thePhonesFleetNamesAPanesTaskFromOpenTasks() throws {
    let worktree = try #require(FleetDecodeTests.decodeFleet().worktrees.first)
    #expect(
        worktree.openTasks == [
            NeedsYouTask(
                id: "0198f2c0-0000-7000-8000-00000000a002", key: "bil-9",
                title: "Invoice PDF export", status: "in_progress")
        ])
    #expect(worktree.openTaskIDs == ["0198f2c0-0000-7000-8000-00000000a002"])
    // The fixture's pane is its workspace's orchestrator, which names none.
    var pane = try #require(worktree.terminals.first)
    pane.taskId = nil
    #expect(pane.isOrchestrator)
    #expect(TaskLink.task(of: pane, in: worktree) == nil)
    pane.role = "shell"
    #expect(TaskLink.task(of: pane, in: worktree) == "0198f2c0-0000-7000-8000-00000000a002")
}

// Which task an agent's own notifications fold into: the runner's
// `task_link::task_of`, less the main checkout (ov-107). Android's
// `TaskLinkTest.kt` states the same cases.

@Test("An agent's notices fold into the task it was opened for, where that task is known")
func anAgentsNoticesFoldIntoTheTaskItWasOpenedFor() {
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: bil7), in: Lane(openTaskIDs: [bil9, bil7])) == bil7)
    // In the main checkout too: it was dispatched there for that task.
    #expect(
        TaskLink.noticeTask(
            of: Pane(boardTaskID: bil7), in: Lane(openTaskIDs: [bil7], isRepositoryCheckout: true)) == bil7)
}

@Test("An agent's own task the client doesn't know folds nothing")
func anAgentsUnknownOwnTaskFoldsNothing() {
    // The runner folds only into a task it still has (`get_task`), and this
    // client can't tell a deleted task from one that's merely elsewhere, so
    // it leaves the banner up: a duplicate at worst, never silence.
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: bil7), in: Lane(openTaskIDs: [])) == nil)
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: bil7), in: Lane(openTaskIDs: [bil9])) == nil)
}

@Test("A lane's task takes in an agent only when its workspace can't differ")
func aLanesTaskTakesInAnAgentOnlyWhenItsWorkspaceCantDiffer() {
    // The runner refuses another workspace's task (task_link.rs `task_of`),
    // and `open_tasks` doesn't say whose a task is.
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: nil, workspace: "ws-b"), in: Lane(openTaskIDs: [bil9])) == nil)
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: nil, workspace: nil), in: Lane(openTaskIDs: [bil9])) == bil9)
}

@Test("An agent opened by hand folds into its lane's one open task, never two")
func anAgentOpenedByHandFoldsIntoItsLanesOneOpenTask() {
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: nil), in: Lane(openTaskIDs: [bil9])) == bil9)
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: ""), in: Lane(openTaskIDs: [bil9])) == bil9)
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: nil), in: Lane(openTaskIDs: [bil9, bil7])) == nil)
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: nil), in: Lane(openTaskIDs: [])) == nil)
}

@Test("The main checkout is nobody's lane for notices, though its header names the task")
func theMainCheckoutIsNobodysLaneForNotices() {
    let checkout = Lane(openTaskIDs: [bil9], isRepositoryCheckout: true)
    let byHand = Pane(boardTaskID: nil)
    #expect(TaskLink.task(of: byHand, in: checkout) == bil9, "the header's rule is unchanged")
    #expect(TaskLink.noticeTask(of: byHand, in: checkout) == nil)
}

@Test("An orchestrator's notices are its own")
func anOrchestratorsNoticesAreItsOwn() {
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: nil, isOrchestrator: true), in: Lane(openTaskIDs: [bil9])) == nil)
    #expect(TaskLink.noticeTask(of: Pane(boardTaskID: bil7, isOrchestrator: true), in: Lane(openTaskIDs: [])) == nil)
}

@Test("A banner is left to the task only where the task's notice arrives")
func aBannerIsLeftToTheTaskOnlyWhereTheNoticeArrives() {
    let agent = Pane(boardTaskID: bil7)
    let loose = Pane(boardTaskID: nil)
    let lane = Lane(openTaskIDs: [bil7, bil9])
    #expect(TaskLink.leavesBannerToTask(agent, in: lane, noticeReachesHere: true))
    #expect(!TaskLink.leavesBannerToTask(agent, in: lane, noticeReachesHere: false), "its own banner is all there is")
    #expect(!TaskLink.leavesBannerToTask(loose, in: lane, noticeReachesHere: true))
}

@Test("A phone hears a task notice only from a paired runner that sends them, once registered")
func aPhoneHearsATaskNoticeOnlyFromAPairedRunnerOnceRegistered() {
    let build = { (capabilities: Set<String>, paired: Bool) in
        DaemonBuild(version: "v", matches: true, platform: "linux", capabilities: capabilities, pushPaired: paired)
    }
    #expect(TaskLink.taskNoticeReachesPhone(build(["tasks", "task_notices"], true), registered: true))
    #expect(!TaskLink.taskNoticeReachesPhone(build(["tasks", "task_notices"], true), registered: false))
    #expect(!TaskLink.taskNoticeReachesPhone(build(["tasks", "task_notices"], false), registered: true))
    #expect(!TaskLink.taskNoticeReachesPhone(build(["tasks"], true), registered: true), "an older runner sends none")
    #expect(!TaskLink.taskNoticeReachesPhone(nil, registered: true))
}

/// The phone's own fleet: the fixture's worktree is the main checkout, with
/// one open task.
@Test("The phone's fleet leaves a hand-opened agent in the main checkout to itself")
func thePhonesFleetLeavesAHandOpenedAgentInTheMainCheckoutToItself() throws {
    let worktree = try #require(FleetDecodeTests.decodeFleet().worktrees.first)
    #expect(worktree.isRepositoryCheckout)
    var pane = try #require(worktree.terminals.first)
    pane.role = "agent"
    pane.taskId = nil
    #expect(TaskLink.noticeTask(of: pane, in: worktree) == nil)
    pane.taskId = "0198f2c0-0000-7000-8000-00000000a002"
    #expect(TaskLink.leavesBannerToTask(pane, in: worktree, noticeReachesHere: true))
}

/// The iPhone's call site: the reports its `Connection` hands `Notifier`,
/// from a decoded fleet. A dispatched agent's banner is left to its task's
/// push only from a paired runner that sends task notices, to a registered
/// phone.
@Test("The iPhone's fleet reports leave a task-bound agent to its task's push")
func theIPhonesFleetReportsLeaveATaskBoundAgentToItsTask() throws {
    var fleet = try FleetDecodeTests.decodeFleet()
    fleet.worktrees[0].terminals[0].role = "agent"
    fleet.worktrees[0].terminals[0].taskId = "0198f2c0-0000-7000-8000-00000000a002"
    let id = fleet.worktrees[0].terminals[0].id
    let paired = DaemonBuild(
        version: "v", matches: true, platform: "linux", capabilities: ["tasks", "task_notices"], pushPaired: true)
    let report = { (build: DaemonBuild?) in
        fleet.agentReports(runner: build, registered: true).first { $0.terminal.id == id }
    }
    #expect(try #require(report(paired)).leftToTask)
    #expect(try #require(report(nil)).leftToTask == false, "no runner build, no push to leave it to")
}

