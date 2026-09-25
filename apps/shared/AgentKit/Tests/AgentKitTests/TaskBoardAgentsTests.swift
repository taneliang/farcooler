import Foundation
import Testing

@testable import AgentKit

// Which agent a card goes to, and what a card says about its acceptance.
//
// The Mac draws both off these answers and decides nothing itself, so what
// could be wrong while the drawing looks right is all here: a pill that takes
// you to a shell prompt where an agent used to be, "No Agent" on a runner too
// old to know, or "0 of 0" on every card with nothing to check.

/// A pane with only the three things the rule reads.
private struct Pane: TaskBoardPane, Equatable {
    var boardTaskID: String?
    var boardState: String = "running"
    var runsAgent: Bool = true
}

private let task = "0198f2c0-0000-7000-8000-00000000a001"
private let other = "0198f2c0-0000-7000-8000-00000000a002"

private func row(
    id: String = task, status: TaskStatus = .inProgress, acceptance: [Bool] = []
) -> TaskRow {
    TaskRow(
        id: id, key: "fc-1", title: "A task", status: status, statusSince: .now,
        acceptance: acceptance.enumerated().map {
            TaskAcceptanceLine(id: "a\($0.offset)", text: "line \($0.offset)", met: $0.element)
        })
}

/// The CLI's `working_on`: the id matches and the pane is running, starting
/// or unknown. Every other state the runner can name is a pane that stopped.
@Test func aPaneWorksItsTaskWhileItIsRunningStartingOrUnknown() {
    for state in ["running", "starting", "unknown"] {
        #expect(
            TaskAgentLink.isWorking(Pane(boardTaskID: task, boardState: state), on: task),
            "\(state) is one of working_on's states")
    }
    for state in ["exited", "error", "LOST", "?", ""] {
        #expect(
            !TaskAgentLink.isWorking(Pane(boardTaskID: task, boardState: state), on: task),
            "\(state) is a pane that stopped")
    }
}

/// Another task's pane, and a pane nobody dispatched, are not this task's.
@Test func onlyAPaneOpenedForTheTaskWorksIt() {
    #expect(!TaskAgentLink.isWorking(Pane(boardTaskID: other), on: task))
    #expect(!TaskAgentLink.isWorking(Pane(boardTaskID: nil), on: task))
    #expect(!TaskAgentLink.isWorking(Pane(boardTaskID: ""), on: ""))
}

/// A dispatched pane whose agent exited is back at its shell, still
/// `running`. Taking someone there from the board would be a link to where
/// the work used to be.
@Test func aPaneThatIsNoLongerRunningAnAgentDoesNotWorkTheTask() {
    #expect(!TaskAgentLink.isWorking(Pane(boardTaskID: task, runsAgent: false), on: task))
}

/// Several agents on one task come back in the order they were handed in,
/// and nothing else on the runner comes with them.
@Test func livePanesAreEveryPaneWorkingTheTaskAndNoOther() {
    let first = Pane(boardTaskID: task)
    let shell = Pane(boardTaskID: task, runsAgent: false)
    let gone = Pane(boardTaskID: task, boardState: "exited")
    let elsewhere = Pane(boardTaskID: other)
    let second = Pane(boardTaskID: task, boardState: "starting")
    #expect(row().livePanes(in: [first, shell, gone, elsewhere, second]) == [first, second])
}

/// An agent on a task is said whatever the task's status is. The one that
/// asked a question moved its card to Needs Decision and is still in its
/// pane, which is exactly when going to it matters.
@Test func aCardWithALiveAgentSaysSoWhateverItsStatus() {
    for status in TaskStatus.allCases {
        #expect(
            row(status: status).agentPresence(livePanes: 1, runnerRecordsTasks: true)
                == .agents(1),
            "\(status) with an agent on it")
    }
    #expect(row().agentPresence(livePanes: 3, runnerRecordsTasks: true) == .agents(3))
}

/// In progress and nobody on it reads "No Agent". Any other status with
/// nobody on it says nothing: a backlog card with no agent is the normal case.
@Test func onlyAnInProgressCardWithNobodyOnItSaysNoAgent() {
    for status in TaskStatus.allCases {
        let presence = row(status: status).agentPresence(livePanes: 0, runnerRecordsTasks: true)
        #expect(presence == (status == .inProgress ? .noAgent : .unsaid), "\(status)")
    }
}

/// A runner without `terminal_task` never puts a task id on a pane, so this
/// app can't know. No link and no remark, rather than a guess.
@Test func aRunnerThatDoesNotRecordTasksGetsNoLinkAndNoRemark() {
    #expect(row().agentPresence(livePanes: 0, runnerRecordsTasks: false) == .unsaid)
    #expect(row().agentPresence(livePanes: 2, runnerRecordsTasks: false) == .unsaid)
}

@Test func thePillSaysAgentForOneAndCountsTheRest() {
    #expect(TaskAgentPresence.agents(1).title == "Agent")
    #expect(TaskAgentPresence.agents(2).title == "2 Agents")
    #expect(TaskAgentPresence.noAgent.title == "No Agent")
    #expect(TaskAgentPresence.unsaid.title == nil)
}

/// Nothing at all for a card with no acceptance, rather than "0 of 0".
@Test func aCardWithNoAcceptanceSaysNothingAboutIt() {
    #expect(row(acceptance: []).acceptanceProgress == nil)
}

@Test func acceptanceReadsKOfNUntilItAllHolds() {
    #expect(row(acceptance: [false]).acceptanceProgress?.sentence == "0 of 1")
    #expect(row(acceptance: [false, false, false]).acceptanceProgress?.sentence == "0 of 3")
    let partial = row(acceptance: [true, false, true, false, false]).acceptanceProgress
    #expect(partial?.sentence == "2 of 5")
    #expect(partial?.isComplete == false)
}

@Test func acceptanceThatAllHoldsSaysSo() {
    let all = row(acceptance: [true, true, true, true, true]).acceptanceProgress
    #expect(all?.sentence == "All 5 Met")
    #expect(all?.isComplete == true)
    let one = row(acceptance: [true]).acceptanceProgress
    #expect(one?.sentence == "Met")
    #expect(one?.isComplete == true)
}

/// Tasks moving, not panes: two agents on one card are one card moving.
@Test func theSidebarCountsTasksWithAnAgentNotAgents() {
    let board = TaskBoardModel.board(from: [])
    #expect(board.tasksWithLiveAgents(in: [Pane(boardTaskID: task)]) == 0)

    let a = row(id: task, status: .inProgress)
    let b = row(id: other, status: .needsDecision)
    let c = row(id: "0198f2c0-0000-7000-8000-00000000a003", status: .todo)
    let filled = TaskBoardModel(columns: [
        TaskBoardColumn(status: .needsDecision, rows: [b]),
        TaskBoardColumn(status: .inProgress, rows: [a]),
        TaskBoardColumn(status: .todo, rows: [c]),
    ])
    let panes = [
        Pane(boardTaskID: task), Pane(boardTaskID: task, boardState: "starting"),
        Pane(boardTaskID: other), Pane(boardTaskID: c.id, boardState: "exited"),
    ]
    #expect(filled.tasksWithLiveAgents(in: panes) == 2)
}

@Test func theSidebarsTooltipsAgreeWithTheirCountAndVanishAtZero() {
    #expect(TaskBoardModel.decisionsHelp(0) == nil)
    #expect(TaskBoardModel.decisionsHelp(1) == "1 task needs a decision")
    #expect(TaskBoardModel.decisionsHelp(2) == "2 tasks need a decision")
    #expect(TaskBoardModel.agentsHelp(0) == nil)
    #expect(TaskBoardModel.agentsHelp(1) == "An agent is on 1 task")
    #expect(TaskBoardModel.agentsHelp(3) == "Agents are on 3 tasks")
}
