import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The breadcrumb's worktree segment after a task's crumb (ov-185): only that
/// task's own worktrees, never the whole workspace's, which stay with a
/// worktree opened straight under the workspace, the navigator's Worktrees
/// section and ⌃⌘↑ ⌃⌘↓.
@MainActor
struct TaskWorktreeCrumbTests {
    private typealias Selection = ContentView.Selection

    private static let billing = "0198f2c0-0000-7000-8000-0000000000dd"
    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"

    private static func terminal(_ id: String, taskId: String? = nil, role: String? = nil, state: String = "running")
        -> Terminal
    {
        var t = Terminal(id: id, short: id, title: id, preset: "claude", state: state, epoch: 0)
        t.taskId = taskId
        t.role = role
        return t
    }

    private static func worktree(_ id: String, terminals: [Terminal] = [], open: [String] = []) -> Worktree {
        var w = Worktree(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: "shop", host: "", path: "/tmp/\(id)",
            state: "active", terminals: terminals, repositoryID: repo, workspace: billing)
        w.openTasks = open.map { NeedsYouTask(id: $0, key: $0, title: $0, status: "in_progress") }
        return w
    }

    /// bil-9 on `pdf` by its own link, with a second agent sent to it in
    /// `pdf-2`; bil-3 on `tax` by an exited agent's pane; bil-5 on `lane`
    /// by the lane's open tasks alone; bil-1 with nothing, beside a scratch
    /// worktree named for it and an orchestrator carrying its id.
    private static func fleet() -> Fleet {
        var fleet = Fleet(
            runtimeHealthy: true, livePanes: 0,
            worktrees: [
                worktree("pdf"),
                worktree("tax", terminals: [terminal("a3", taskId: "t3", state: "exited")]),
                worktree("pdf-2", terminals: [terminal("a9", taskId: "t9")]),
                worktree("lane", open: ["t5"]),
                worktree("bil-1", terminals: [terminal("o", taskId: "t1", role: "orchestrator")]),
                worktree("scratch"),
            ],
            branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [
            WorkspaceSummary(id: billing, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: repo)
        ]
        return fleet
    }

    private static func row(_ id: String, _ key: String, worktree: String? = nil) -> TaskRow {
        TaskRow(id: id, key: key, title: "Task \(key)", status: .inProgress, statusSince: .now, worktreeID: worktree)
    }

    private static let rows = [
        row("t9", "bil-9", worktree: "pdf"), row("t3", "bil-3"), row("t5", "bil-5"), row("t1", "bil-1"),
    ]
    private static let board = TaskBoardModel(columns: [TaskBoardColumn(status: .inProgress, rows: rows)])

    private static func task(_ id: String) -> Selection { .workspace(host: "", workspace: billing, focus: .task(id)) }
    private static func whole(_ id: String) -> Selection {
        .workspace(host: "", workspace: billing, focus: .worktree(id, terminal: nil))
    }

    private static func segment(_ place: Selection, trail: Selection? = nil) -> WorkspaceWorktrees.Segment? {
        let fleet = fleet()
        let summary = fleet.runnerWorkspaces[""]![0]
        return WorkspaceWorktrees.segment(
            for: place, trail: trail,
            entries: WorkspaceWorktrees.entries(in: summary, host: "", board: board, fleet: fleet),
            taskWorktrees: { id in
                rows.first { $0.id == id }.map { WorkspaceWorktrees.worktrees(of: $0, host: "", in: fleet) } ?? []
            },
            name: { _, id in id }, host: "", workspace: billing, fleet: fleet)
    }

    @Test("A task's worktrees: its own first, then any other with its lane or a pane for it, in the runner's order; never by name")
    func aTasksWorktrees() {
        let fleet = Self.fleet()
        func ids(_ row: TaskRow) -> [String] { WorkspaceWorktrees.worktrees(of: row, host: "", in: fleet).map(\.id) }
        #expect(ids(Self.rows[0]) == ["pdf", "pdf-2"])
        // A pane opened for it counts after it's exited: the work is there.
        #expect(ids(Self.rows[1]) == ["tax"])
        #expect(ids(Self.rows[2]) == ["lane"])
        // Not the worktree named for it, nor the orchestrator carrying its id.
        #expect(ids(Self.rows[3]) == [])
    }

    @Test("A task with no worktree has no segment, so no menu")
    func noWorktreeNoMenu() {
        #expect(Self.segment(Self.task("t1")) == nil)
    }

    @Test("A task with one worktree names it, and opens it with the task as the way back")
    func oneWorktreeOpensIt() throws {
        let segment = try #require(Self.segment(Self.task("t3")))
        #expect(segment.title == "tax")
        #expect(segment.isHere == false && segment.isWorkspace == false)
        #expect(segment.tasks.isEmpty && segment.loose.isEmpty)
        #expect(segment.opens?.target == Self.whole("tax"))
        #expect(segment.opens?.trail == Self.task("t3"))
        let pieces = DrillBreadcrumb.pieces(
            [.init(title: "Billing", target: nil)],
            worktrees: WorktreeCrumb(title: segment.title, isHere: false, tasks: [], loose: [], opens: segment.opens))
        #expect(!pieces.contains { $0.kind == .menuChevron }, "a way in, not a menu, has no ⌄")
    }

    @Test("A task with several worktrees lists only those")
    func severalWorktreesListOnlyThose() throws {
        let segment = try #require(Self.segment(Self.task("t9")))
        #expect(segment.opens == nil)
        #expect(segment.tasks.isEmpty)
        #expect(segment.loose.map(\.title) == ["pdf", "pdf-2"])
        #expect(segment.loose.map(\.target) == [Self.whole("pdf"), Self.whole("pdf-2")])
        #expect(segment.loose.allSatisfy { $0.trail == Self.task("t9") && !$0.current })
    }

    @Test("A worktree opened from its task lists the task's, itself checked")
    func openedFromItsTask() throws {
        let segment = try #require(Self.segment(Self.whole("pdf-2"), trail: Self.task("t9")))
        #expect(segment.isHere && !segment.isWorkspace)
        #expect(segment.title == "pdf-2")
        #expect(segment.loose.map(\.title) == ["pdf", "pdf-2"])
        #expect(segment.loose.map(\.current) == [false, true])
    }

    @Test("A worktree opened under the workspace still lists every worktree in it")
    func theWorkspaceLevelListsAll() throws {
        let segment = try #require(Self.segment(Self.whole("scratch")))
        #expect(segment.isHere && segment.isWorkspace)
        #expect(segment.title == "scratch")
        let all = (segment.tasks + segment.loose).map(\.target)
        // The navigator's order: the task its row names a worktree for,
        // then the rest, `scratch` among them.
        #expect(segment.tasks.map(\.title) == ["bil-9 Task bil-9"])
        #expect(segment.loose.map(\.title) == ["tax", "pdf-2", "lane", "bil-1", "scratch"])
        #expect(all.count == 6)
        #expect(segment.loose.first { $0.title == "scratch" }?.current == true)
        #expect(Self.segment(.workspace(host: "", workspace: Self.billing, focus: nil)) == nil)
    }

    /// **A task whose agent exited says so** (ov-185 review, point 5): the
    /// crumb offers the worktree it ran in, so the Overview mustn't say
    /// nothing has started. Start Agent… is still offered. Not for an
    /// orchestrator carrying the task's id, nor a task never started.
    @Test("A task whose only agent exited says its agent stopped, not that nothing started")
    func anExitedAgentSaysItStopped() {
        let fleet = Self.fleet()
        #expect(TaskColumnModel.hadAgent("t3", host: "", in: fleet))
        #expect(!TaskColumnModel.hadAgent("t1", host: "", in: fleet), "an orchestrator is no task's agent")
        #expect(!TaskColumnModel.hadAgent("t5", host: "", in: fleet), "a lane listing it ran no agent")
        #expect(!TaskColumnModel.hadAgent("t3", host: "elsewhere", in: fleet))

        let stopped = TaskColumnModel.agent(hasAgent: false, worktree: nil, stopped: true)
        #expect(stopped == .stopped)
        #expect(TaskColumnModel.sentence(stopped) == "This task’s agent has stopped.")
        #expect(TaskColumnModel.offersStart(status: .inProgress, worktree: false, agent: stopped, offersWrites: true))
        // A worktree of its own still wins: Open Worktree, as before.
        #expect(TaskColumnModel.agent(hasAgent: false, worktree: "w", stopped: true) == .none(openWorktree: true))
        #expect(TaskColumnModel.agent(hasAgent: true, worktree: nil, stopped: true) == .live)
    }

    /// `WorkspaceWorktrees.crumbs(_:isHere:)`, which ContentView draws
    /// before the segment: the last crumb is dropped only when the segment
    /// stands for it.
    @Test("The crumbs before the segment drop the last only when the segment is it")
    func theCrumbsBeforeTheSegment() {
        let crumbs = [WorkspaceNavigation.Crumb(title: "Billing", target: nil), .init(title: "scratch", target: nil)]
        #expect(WorkspaceWorktrees.crumbs(crumbs, isHere: true).map(\.title) == ["Billing"])
        #expect(WorkspaceWorktrees.crumbs(crumbs, isHere: false).map(\.title) == ["Billing", "scratch"])
    }
}
