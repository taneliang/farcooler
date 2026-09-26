import Foundation
import Testing

@testable import AgentKit

// Which repositories get a Board row in the phone's overview, and what the row
// says. The overview draws these and decides nothing, so a row for an empty
// board, a count from a runner that cannot be believed, or a row on a runner
// with no board at all would all be decided here or nowhere.

private struct Pane: TaskBoardPane {
    var boardTaskID: String?
    var boardState: String = "running"
    var runsAgent: Bool = true
}

private func row(_ id: String, _ status: TaskStatus) -> TaskRow {
    TaskRow(id: id, key: "-\(id)", title: "Task \(id)", status: status, statusSince: .now)
}

private func board(_ rows: [TaskRow], unreadable: [UnreadableTaskRow] = []) -> TaskBoardModel {
    TaskBoardModel(
        columns: TaskBoardModel.order.map { status in
            TaskBoardColumn(status: status, rows: rows.filter { $0.status == status })
        },
        unreadable: unreadable)
}

private let both = DaemonBuild(
    version: "1", matches: true, platform: "", capabilities: ["workspaces", "tasks", "terminal_task"])

private let repositories: [(id: String, name: String)] = [
    (id: "r-busy", name: "overnight"), (id: "r-empty", name: "scratch"),
    (id: "r-unread", name: "never-read"), (id: "r-new", name: "newer"),
]

private let boards: [String: TaskBoardModel] = [
    "r-busy": board([
        row("1", .needsDecision), row("2", .needsDecision), row("3", .inProgress),
        row("4", .inProgress), row("5", .done),
    ]),
    "r-empty": board([]),
    "r-new": board([], unreadable: [UnreadableTaskRow(id: "9", key: "-9", title: "x", status: "parked")]),
]

/// Two agents on task 3 and one on task 4: two tasks moving, three panes.
private let panes = [
    Pane(boardTaskID: "3"), Pane(boardTaskID: "3"), Pane(boardTaskID: "4"),
    Pane(boardTaskID: "5", boardState: "exited"),
]

/// A row for a repository whose board has something on it, and for no other:
/// not an empty board, and not one nobody has read yet. A board holding only
/// rows this build cannot place still has something on it.
@Test func onlyARepositoryWithSomethingOnItsBoardGetsARow() {
    let rows = RunnerBoards.rows(
        repositories: repositories, boards: boards, panes: panes, build: both, connected: true)
    #expect(rows.map(\.repository) == ["r-busy", "r-new"])
    #expect(rows.first?.name == "overnight")
}

/// The two numbers: tasks in Needs Decision, and tasks (not panes) an agent is
/// on.
@Test func aRowCountsDecisionsAndTheTasksAgentsAreOn() throws {
    let rows = RunnerBoards.rows(
        repositories: repositories, boards: boards, panes: panes, build: both, connected: true)
    let busy = try #require(rows.first)
    #expect(busy.decisions == 2)
    #expect(busy.agents == 2)
    #expect(busy.spoken == "2 tasks need a decision, Agents are on 2 tasks")
}

/// A runner that is not connected says nothing about agents, and neither does
/// one that does not record which pane works which task. The decisions are
/// what the board said and stay.
@Test func aRunnerThatCannotBeBelievedAboutItsPanesCountsNoAgents() throws {
    let reconnecting = RunnerBoards.rows(
        repositories: repositories, boards: boards, panes: panes, build: both, connected: false)
    #expect(try #require(reconnecting.first).agents == 0)
    #expect(try #require(reconnecting.first).decisions == 2)

    let older = DaemonBuild(
        version: "1", matches: true, platform: "", capabilities: ["workspaces", "tasks"])
    let unrecorded = RunnerBoards.rows(
        repositories: repositories, boards: boards, panes: panes, build: older, connected: true)
    #expect(try #require(unrecorded.first).agents == 0)
}

/// No board on a runner without `tasks`, and none before the runner has been
/// asked what it can do.
@Test func aRunnerWithoutABoardGetsNoRows() {
    let old = DaemonBuild(version: "1", matches: true, platform: "", capabilities: ["workspaces"])
    #expect(
        RunnerBoards.rows(
            repositories: repositories, boards: boards, panes: panes, build: old, connected: true
        ).isEmpty)
    #expect(
        RunnerBoards.rows(
            repositories: repositories, boards: boards, panes: panes, build: nil, connected: true
        ).isEmpty)
    // Silence is a runner older than capabilities, which has no board either.
    let silent = DaemonBuild(version: "1", matches: true, platform: "")
    #expect(
        RunnerBoards.rows(
            repositories: repositories, boards: boards, panes: panes, build: silent,
            connected: true
        ).isEmpty)
}

/// Nothing to say is said as nothing.
@Test func aRowWithNothingToCountSaysNothing() {
    #expect(RunnerBoardRow(repository: "r", name: "n", decisions: 0, agents: 0).spoken == nil)
    #expect(
        RunnerBoardRow(repository: "r", name: "n", decisions: 1, agents: 0).spoken
            == "1 task needs a decision")
}

/// The phone lists the statuses with tasks in them, Needs Decision first and
/// the rest in the order work moves.
@Test func thePhoneListsOnlyTheStatusesWithTasksNeedsDecisionFirst() throws {
    let listed = try #require(boards["r-busy"]).listed
    #expect(listed.map(\.status) == [.needsDecision, .inProgress, .done])
    #expect(board([]).listed.isEmpty)
}

/// A runner whose new link has not read its build yet keeps the rows the
/// last build allowed, so nothing under them moves — and says nothing about
/// agents until the fresh build lands.
@Test func aReconnectedRunnerKeepsItsRowsWhileItsBuildIsReadAgain() throws {
    let gap = RunnerBoards.rows(
        repositories: repositories, boards: boards, panes: panes,
        build: nil, lastKnownBuild: both, connected: true)
    #expect(gap.map(\.repository) == ["r-busy", "r-new"])
    #expect(try #require(gap.first).decisions == 2)
    #expect(try #require(gap.first).agents == 0)

    let landed = RunnerBoards.rows(
        repositories: repositories, boards: boards, panes: panes,
        build: both, lastKnownBuild: both, connected: true)
    #expect(try #require(landed.first).agents == 2)
}
