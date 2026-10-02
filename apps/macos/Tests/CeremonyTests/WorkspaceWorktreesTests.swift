import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Getting around a workspace's worktrees with no sidebar (ov-86): the order
/// ⌃⌘↓ and ⌃⌘↑ walk, the breadcrumb's worktree menu, the board list's
/// Worktrees section, ⌘1 through ⌘9, and the sidebar's default.
@MainActor
struct WorkspaceWorktreesTests {
    private typealias Selection = ContentView.Selection

    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let other = "0198f2c0-0000-7000-8000-0000000000ab"
    private static let main = "0198f2c0-0000-7000-8000-0000000000cc"
    private static let billing = "0198f2c0-0000-7000-8000-0000000000dd"
    private static let relayMain = "0198f2c0-0000-7000-8000-0000000000ee"

    private static func terminal(
        _ id: String, preset: String = "zsh", taskId: String? = nil, role: String? = nil, workspace: String? = nil
    ) -> Terminal {
        var t = Terminal(id: id, short: id, title: id, preset: preset, state: "running", epoch: 0)
        t.taskId = taskId
        t.role = role
        t.workspace = workspace
        return t
    }

    private static func worktree(
        _ id: String, workspace: String?, repository: String = repo, state: String = "active",
        terminals: [Terminal] = [], main: Bool = false
    ) -> Worktree {
        var w = Worktree(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: repository == repo ? "shop" : "relay",
            host: "", path: "/tmp/\(id)", state: state, terminals: terminals, repositoryID: repository,
            workspace: workspace)
        w.is_main_checkout = main
        return w
    }

    private static let workspaces = [
        WorkspaceSummary(id: main, name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: repo),
        WorkspaceSummary(
            id: billing, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: repo,
            orchestrator: "conductor"),
        WorkspaceSummary(id: relayMain, name: "Main", taskPrefix: "rl", isMain: true, ordinal: 0, repository: other),
    ]

    /// The main checkout, holding Billing's orchestrator and a shell; two
    /// task lanes and a scratch worktree in Billing, one hidden; an
    /// unclaimed worktree; and another repository's checkout.
    private static func fleet() -> Fleet {
        var fleet = Fleet(
            runtimeHealthy: true, livePanes: 0,
            worktrees: [
                worktree(
                    "checkout", workspace: main,
                    terminals: [
                        terminal("conductor", preset: "claude", role: "orchestrator", workspace: billing),
                        terminal("shell"),
                    ], main: true),
                worktree("tax", workspace: billing, terminals: [terminal("a1", preset: "claude", taskId: "t3")]),
                worktree("pdf", workspace: billing),
                worktree("scratch", workspace: billing, terminals: [terminal("s1")]),
                worktree("old", workspace: billing, state: "hidden"),
                worktree("stray", workspace: nil),
                worktree("relay-checkout", workspace: relayMain, repository: other, main: true),
            ],
            branchPrefix: nil)
        fleet.runnerWorkspaces[""] = workspaces
        return fleet
    }

    private static func row(_ id: String, _ key: String, _ status: TaskStatus, worktree: String? = nil) -> TaskRow {
        TaskRow(id: id, key: key, title: "Task \(key)", status: status, statusSince: .now, worktreeID: worktree)
    }

    /// Billing's board: bil-9 (In Progress) on `pdf` by its own link, bil-3
    /// (Needs Decision, so listed first) on `tax` by its agent alone, and
    /// bil-1 with no worktree.
    private static let board = TaskBoardModel(columns: [
        TaskBoardColumn(status: .inProgress, rows: [row("t9", "bil-9", .inProgress, worktree: "pdf")]),
        TaskBoardColumn(status: .needsDecision, rows: [row("t3", "bil-3", .needsDecision)]),
        TaskBoardColumn(status: .todo, rows: [row("t1", "bil-1", .todo)]),
    ])

    private static var billingSummary: WorkspaceSummary { workspaces[1] }

    private static func entries() -> [WorkspaceWorktrees.Entry] {
        WorkspaceWorktrees.entries(in: billingSummary, host: "", board: board, fleet: fleet())
    }

    // MARK: - Order, next and previous

    @Test("Worktrees go in board-list order: tasks' by the board's sections, then the loose ones")
    func theOrderIsTheBoardList() {
        let order = Self.entries().map { "\($0.worktree.id):\($0.task?.key ?? "-")" }
        #expect(order == ["tax:bil-3", "pdf:bil-9", "scratch:-"])
    }

    @Test("⌃⌘↓ and ⌃⌘↑ step from the open task or worktree, and wrap")
    func nextAndPreviousStepAndWrap() {
        let entries = Self.entries()
        func step(_ from: Selection?, _ by: Int) -> Selection? {
            WorkspaceWorktrees.step(from: from, by: by, in: entries, host: "", workspace: Self.billing, fleet: Self.fleet())
        }
        let tax = Selection.workspace(host: "", workspace: Self.billing, focus: .task("t3"))
        let pdf = Selection.workspace(host: "", workspace: Self.billing, focus: .task("t9"))
        let scratch = Selection.workspace(host: "", workspace: Self.billing, focus: .worktree("scratch", terminal: nil))
        let board = Selection.workspace(host: "", workspace: Self.billing, focus: nil)
        #expect(step(tax, 1) == pdf)
        #expect(step(pdf, 1) == scratch)
        #expect(step(scratch, 1) == tax)
        #expect(step(tax, -1) == scratch)
        #expect(step(scratch, -1) == pdf)
        // From the board alone: the first going down, the last going up.
        #expect(step(board, 1) == tax)
        #expect(step(board, -1) == scratch)
        // A task's worktree opened whole counts as where its task is.
        #expect(step(.workspace(host: "", workspace: Self.billing, focus: .worktree("pdf", terminal: "x")), 1) == scratch)
        // A task with no worktree isn't in the order: going on starts over.
        #expect(step(.workspace(host: "", workspace: Self.billing, focus: .task("t1")), 1) == tax)
    }

    @Test("Stepping with one worktree, or none, goes nowhere")
    func steppingNowhere() {
        let one = Array(Self.entries().prefix(1))
        let tax = Selection.workspace(host: "", workspace: Self.billing, focus: .task("t3"))
        #expect(WorkspaceWorktrees.step(from: tax, by: 1, in: one, host: "", workspace: Self.billing, fleet: Self.fleet()) == nil)
        #expect(WorkspaceWorktrees.step(from: tax, by: 1, in: [], host: "", workspace: Self.billing, fleet: Self.fleet()) == nil)
    }

    // MARK: - The breadcrumb's menu

    @Test("The breadcrumb's menu lists task worktrees by their task, then the loose ones, the current one checked")
    func theBreadcrumbMenu() {
        let here = Selection.workspace(host: "", workspace: Self.billing, focus: .task("t9"))
        let menu = WorkspaceWorktrees.menu(
            Self.entries(), selection: here, host: "", workspace: Self.billing, fleet: Self.fleet())
        #expect(menu.tasks.map(\.title) == ["bil-3 Task bil-3", "bil-9 Task bil-9"])
        #expect(menu.tasks.map(\.subtitle) == ["⎇ tax", "⎇ pdf"])
        #expect(menu.tasks.map(\.current) == [false, true])
        #expect(menu.tasks.map(\.target) == [
            .workspace(host: "", workspace: Self.billing, focus: .task("t3")), here,
        ])
        #expect(menu.loose.map(\.title) == ["⎇ scratch"])
        #expect(menu.loose.map(\.target) == [
            .workspace(host: "", workspace: Self.billing, focus: .worktree("scratch", terminal: nil))
        ])
        #expect(menu.loose.map(\.current) == [false])
    }

    // MARK: - The Worktrees section

    @Test("The Worktrees section holds only loose worktrees, hidden ones apart")
    func theSectionIsLooseOnly() {
        let loose = WorkspaceWorktrees.loose(in: Self.billingSummary, host: "", board: Self.board, fleet: Self.fleet())
        #expect(loose.shown.map(\.id) == ["scratch"])
        #expect(loose.hidden.map(\.id) == ["old"])
    }

    @Test("Main's section holds the main checkout and unclaimed worktrees, never the orchestrator's terminal")
    func mainsSectionHasNoOrchestrator() {
        let loose = WorkspaceWorktrees.loose(in: Self.workspaces[0], host: "", board: .empty, fleet: Self.fleet())
        #expect(loose.shown.map(\.id) == ["checkout", "stray"])
        let checkout = loose.shown.first { $0.id == "checkout" }
        #expect(checkout?.terminals.map(\.id) == ["shell"])
    }

    // MARK: - ⌘1 through ⌘9

    @Test("⌘-numbers go to workspaces in the switcher's order, grouped by repository")
    func numbersFollowTheSwitcher() {
        let groups = WorkspaceNumbers.groups(in: Self.fleet())
        #expect(groups.map(\.repository) == ["shop", "relay"])
        #expect(groups.flatMap(\.places).map(\.name) == ["Main", "Billing", "Main"])
        #expect(groups.flatMap(\.places).map(\.number) == [1, 2, 3])
        #expect(WorkspaceNumbers.target(2, in: groups) == .workspace(host: "", workspace: Self.billing, focus: nil))
        #expect(WorkspaceNumbers.target(3, in: groups) == .workspace(host: "", workspace: Self.relayMain, focus: nil))
        #expect(WorkspaceNumbers.target(4, in: groups) == nil)
    }

    @Test("Only the first nine workspaces have a ⌘-number")
    func onlyNineNumbers() {
        var fleet = Self.fleet()
        fleet.runnerWorkspaces[""] = Self.workspaces + (2...10).map { n in
            WorkspaceSummary(
                id: "ws-\(n)", name: "W\(n)", taskPrefix: "w\(n)", isMain: false, ordinal: n, repository: Self.repo)
        }
        let places = WorkspaceNumbers.groups(in: fleet).flatMap(\.places)
        #expect(places.count == 12)
        #expect(places.compactMap(\.number) == Array(1...9))
        #expect(places.last?.number == nil)
    }

    // MARK: - The sidebar's default

    @Test("A new window starts without the sidebar; one used before keeps it")
    func sidebarDefault() {
        #expect(SidebarDefault.shown(stored: nil, hasHistory: false) == false)
        #expect(SidebarDefault.shown(stored: nil, hasHistory: true) == true)
        #expect(SidebarDefault.shown(stored: "hidden", hasHistory: true) == false)
        #expect(SidebarDefault.shown(stored: "shown", hasHistory: false) == true)
    }
}
