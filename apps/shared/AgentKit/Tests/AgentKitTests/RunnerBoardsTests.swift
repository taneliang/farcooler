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

/// A runner whose new link has not read its build yet keeps the rows the
/// last build allowed, so nothing under them moves — and says nothing about
/// agents until the fresh build lands.
@Test func aReconnectedRunnerKeepsItsRowsWhileItsBuildIsReadAgain() throws {
    let gap = RunnerBoards.rows(
        repositories: repositories, boards: boards, panes: panes,
        build: nil, lastKnownBuild: both, connected: true)
    #expect(gap.map(\.repository) == ["r-busy", "r-empty", "r-unread", "r-new"])
    #expect(try #require(gap.first).decisions == 2)
    #expect(try #require(gap.first).agents == 0)

    let landed = RunnerBoards.rows(
        repositories: repositories, boards: boards, panes: panes,
        build: both, lastKnownBuild: both, connected: true)
    #expect(try #require(landed.first).agents == 2)
}

/// A link whose sweep was refused for want of a build owes one when the build
/// lands; one whose boards were read does not; and a new link owes nothing
/// until its own sweep is refused. The case this exists for: the first
/// `host` read on a reconnect failed, the sweep that followed refused every
/// board, and a later poll installed the build.
@Test func aBuildThatLandsLateReadsTheBoardsItsLinkNeverRead() {
    var sweep = BoardSweep()
    sweep.linkCameUp()
    sweep.refused()
    #expect(sweep.owedWhenBuildLands, "the sweep on this link read nothing")
    sweep.swept()
    #expect(!sweep.owedWhenBuildLands, "this link already read its boards")
    sweep.linkCameUp()
    sweep.refused()
    #expect(sweep.owedWhenBuildLands, "a new link has read nothing")
}

/// **A build landing on the ordinary path starts no sweep of its own** (ov-20
/// R-M8). Every link-up reads the build first and then sweeps; the build
/// landing before that sweep started one too, so every connect and reconnect
/// read every board twice. Only a sweep already refused is owed.
///
/// Mutation: `owedWhenBuildLands` back to `!sweptOnThisLink`. Red.
@Test func aBuildLandingBeforeTheSweepStartsNoSecondSweep() {
    var sweep = BoardSweep()
    sweep.linkCameUp()
    #expect(!sweep.owedWhenBuildLands, "no sweep has been refused on this link")
}

/// **A sweep stops at the link it was started on** (ov-20 R-M8). A sweep
/// still going when its link dropped used to carry on over the new link,
/// beside the new link's own sweep, and every board they shared cost a read
/// and a trailing re-read.
///
/// Mutation: `isCurrent` answering true for any link. Red.
@Test func aSweepFromAnEarlierLinkIsNotCurrent() {
    var sweep = BoardSweep()
    sweep.linkCameUp()
    let first = sweep.link
    #expect(sweep.isCurrent(first))
    sweep.linkCameUp()
    #expect(!sweep.isCurrent(first), "the link it started on is gone")
    #expect(sweep.isCurrent(sweep.link))
}

// MARK: - Boards by workspace

private func workspace(
    _ id: String, _ name: String, in repository: String, main: Bool = false, ordinal: Int = 0
) -> WorkspaceSummary {
    WorkspaceSummary(
        id: id, name: name, taskPrefix: id, isMain: main, ordinal: ordinal,
        repository: repository)
}

/// A repository split in two is two boards, and so two rows: keyed by the
/// workspace, called by the workspace's name, each counting its own board.
@Test func twoWorkspacesInOneRepositoryAreTwoBoardRows() {
    let main = workspace("w-main", "Main", in: "r-busy", main: true)
    let billing = workspace("w-billing", "Billing", in: "r-busy", ordinal: 1)
    let rows = RunnerBoards.rows(
        boards: [main, billing], names: ["r-busy": "overnight"],
        models: [
            "w-main": board([row("1", .needsDecision), row("3", .inProgress)]),
            "w-billing": board([row("2", .needsDecision), row("6", .needsDecision)]),
        ],
        panes: panes, build: both, connected: true)
    #expect(rows.map(\.id) == ["w-main", "w-billing"])
    #expect(rows.map(\.name) == ["Main", "Billing"])
    #expect(rows.map(\.decisions) == [1, 2])
    #expect(rows.map(\.repository) == ["r-busy", "r-busy"])
    // Read by workspace, not as the whole repository.
    #expect(rows.map(\.workspace.boardWorkspace) == ["w-main", "w-billing"])
}

/// **A workspace's board has a row as soon as the workspace exists** (ov-56),
/// empty or not read yet. A workspace made a moment ago has nothing on its
/// board, and a row only for a board with something on it left the new
/// workspace a heading whose board could not be opened. The board draws the
/// empty and unread states. Android's `aWorkspacesBoardHasARowWhileEmptyOrUnread`.
@Test func aWorkspacesBoardHasARowWhileEmptyOrUnread() {
    let main = workspace("w-main", "Main", in: "r", main: true)
    let billing = workspace("w-billing", "Billing", in: "r", ordinal: 1)
    let rows = RunnerBoards.rows(
        boards: [main, billing], names: [:],
        models: ["w-main": board([])],
        panes: panes, build: both, connected: true)
    #expect(rows.map(\.id) == ["w-main", "w-billing"])
    #expect(rows.map(\.decisions) == [0, 0])
    #expect(rows.map(\.agents) == [0, 0])
}

/// What a sweep reads — a link coming up, a reconnect: every workspace's
/// board, Main first within its repository, and the repository's whole board
/// where the runner names no workspace for it. An open board is one of these,
/// which is what reads it again after a reconnect.
@Test func aSweepReadsEveryWorkspacesBoardAndTheWholeRepositoryWhereThereAreNone() {
    let billing = workspace("w-billing", "Billing", in: "r1", ordinal: 1)
    let main = workspace("w-main", "Main", in: "r1", main: true)
    let elsewhere = workspace("w-late", "Main", in: "r3", main: true)
    let swept = RunnerBoards.boards(
        repositories: ["r1", "r2"], workspaces: [billing, main, elsewhere])
    #expect(swept.map(\.id) == ["w-main", "w-billing", "r2", "w-late"])
    #expect(swept.map(\.boardWorkspace) == ["w-main", "w-billing", nil, "w-late"])

    // A runner without workstreams: one implicit board per repository, read
    // whole — exactly the sweep from before workspaces.
    let old = RunnerBoards.boards(repositories: ["r1", "r2"], workspaces: nil)
    #expect(old.map(\.id) == ["r1", "r2"])
    #expect(old.allSatisfy { $0.boardWorkspace == nil })
}

/// A notice reads the board it names and the board it left, not the others —
/// and a board the fleet has not listed yet is read anyway, so its row is
/// there the moment the fleet names its workspace.
@Test func aNoticeReadsItsOwnBoardsAndOneTheFleetHasNotListedYet() {
    let main = workspace("w-main", "Main", in: "r1", main: true)
    let billing = workspace("w-billing", "Billing", in: "r1", ordinal: 1)
    let held = [main, billing]

    let own = BoardNotice(repository: "r1", workspace: "w-billing")
    #expect(RunnerBoards.touched(by: own, among: held).map(\.id) == ["w-billing"])

    let move = BoardNotice(repository: "r1", workspace: "w-main", fromWorkspace: "w-billing")
    #expect(RunnerBoards.touched(by: move, among: held).map(\.id) == ["w-main", "w-billing"])

    let new = BoardNotice(repository: "r1", workspace: "w-new")
    let read = RunnerBoards.touched(by: new, among: held)
    #expect(read.map(\.id) == ["w-new"])
    #expect(read.first?.boardWorkspace == "w-new")
    #expect(read.first?.repository == "r1")

    // A runner without workstreams names no workspace: its repository's board.
    let implicit = [WorkspaceSummary.implicit(repository: "r1"), .implicit(repository: "r2")]
    let old = BoardNotice(repository: "r2", workspace: nil)
    #expect(RunnerBoards.touched(by: old, among: implicit).map(\.id) == ["r2"])
}

/// An implicit board is the whole repository's, so a notice that names a
/// workspace still reads it: a runner upgraded under a connected app sends
/// workspaces before the fleet read that would key the boards by them.
@Test func anImplicitBoardReadsForANoticeThatNamesAWorkspace() {
    let implicit = [WorkspaceSummary.implicit(repository: "r1"), .implicit(repository: "r2")]
    let named = BoardNotice(repository: "r1", workspace: "w-main")
    let read = RunnerBoards.touched(by: named, among: implicit)
    #expect(read.map(\.id) == ["r1", "w-main"])
    #expect(read.first?.boardWorkspace == nil, "the implicit board is read whole")
}

// MARK: - Sections and the implicit board's row (ov-55, spec §5)

/// **Every status is a section, the empty ones included** (owner decision 3).
/// The list form draws an empty status as a collapsed header reading
/// "Backlog 0"; a list that dropped it would say nothing about what isn't
/// there. Every status in `order`, Needs Decision first, each with its
/// count, whether the board was built with all seven columns, some, or none.
@Test("Every status is a section, empty ones included")
func everyStatusIsASectionEmptyOnesIncluded() throws {
    let busy = try #require(boards["r-busy"])
    #expect(busy.sections.map(\.status) == TaskBoardModel.order)
    #expect(busy.sections.map(\.count) == [2, 0, 0, 2, 0, 1, 0])
    #expect(busy.sections.map(\.title).first == "Needs Decision")
    // A board with no columns at all, which is what `.empty` is.
    #expect(TaskBoardModel.empty.sections.map(\.status) == TaskBoardModel.order)
    #expect(TaskBoardModel.empty.sections.allSatisfy { $0.count == 0 })
    // A board built with only some of its columns keeps their rows.
    let partial = TaskBoardModel(columns: [
        TaskBoardColumn(status: .done, rows: [row("5", .done)])
    ])
    #expect(partial.sections.map(\.count) == [0, 0, 0, 0, 0, 1, 0])
    #expect(partial.sections.first { $0.status == .done }?.rows.map(\.id) == ["5"])
}

/// **An empty implicit board still has a row** (spec §5, §8). An implicit
/// board is a repository's on a runner without `workstreams`, and the
/// workspace view treats it as a workspace, so an empty one has to be
/// reachable: the row is the way onto the board, and New Task… is on the
/// board. Read or not, like a workspace's (ov-56).
@Test("An empty implicit board still has a row")
func anEmptyImplicitBoardStillHasARow() {
    let rows = RunnerBoards.rows(
        repositories: repositories, boards: boards, panes: panes, build: both, connected: true)
    #expect(rows.map(\.repository) == ["r-busy", "r-empty", "r-unread", "r-new"])
    #expect(rows.map(\.name) == ["overnight", "scratch", "never-read", "newer"])
    #expect(rows.map(\.decisions) == [2, 0, 0, 0])
}
