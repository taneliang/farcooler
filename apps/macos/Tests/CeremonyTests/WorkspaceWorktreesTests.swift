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
        #expect(order == ["tax:bil-3", "pdf:bil-9", "scratch:-", "old:-"])
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
        let old = Selection.workspace(host: "", workspace: Self.billing, focus: .worktree("old", terminal: nil))
        #expect(step(pdf, 1) == scratch)
        #expect(step(scratch, 1) == old)
        #expect(step(old, 1) == tax)
        #expect(step(tax, -1) == old)
        #expect(step(scratch, -1) == pdf)
        // From the board alone: the first going down, the last going up.
        #expect(step(board, 1) == tax)
        #expect(step(board, -1) == old)
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
        #expect(menu.tasks.map(\.subtitle) == ["tax", "pdf"])
        #expect(menu.tasks.map(\.current) == [false, true])
        #expect(menu.tasks.map(\.target) == [
            .workspace(host: "", workspace: Self.billing, focus: .task("t3")), here,
        ])
        #expect(menu.loose.map(\.title) == ["scratch", "old"])
        #expect(menu.loose.map(\.target) == [
            .workspace(host: "", workspace: Self.billing, focus: .worktree("scratch", terminal: nil)),
            .workspace(host: "", workspace: Self.billing, focus: .worktree("old", terminal: nil)),
        ])
        #expect(menu.loose.map(\.current) == [false, false])
        // Each item is its place: two worktrees of one name stay two.
        #expect(Set(menu.loose.map(\.id)).count == 2)
    }

    /// The order is the list's as drawn (review M2): Done newest first,
    /// not in the runner's order, a collapsed section's rows where they'd
    /// be drawn, and Done's older tasks, which the list cuts, after its
    /// recent ones.
    @Test("⌃⌘↓ walks the rows in the order the list draws them, Done newest first")
    func theOrderIsAsDrawn() {
        var fleet = Self.fleet()
        fleet.worktrees += ["d1", "d2", "d3", "c1"].map { Self.worktree($0, workspace: Self.billing) }
        let now = Date()
        func done(_ id: String, _ key: String, daysAgo: Double, on worktree: String) -> TaskRow {
            TaskRow(
                id: id, key: key, title: key, status: .done, statusSince: now.addingTimeInterval(-daysAgo * 86_400),
                worktreeID: worktree)
        }
        // In the runner's order: oldest first. Twelve more Done tasks, all
        // recent, push the oldest past what the list shows.
        let filler = (0..<12).map { n in
            TaskRow(id: "f\(n)", key: "bil-f\(n)", title: "", status: .done, statusSince: now.addingTimeInterval(-60))
        }
        let board = TaskBoardModel(columns: [
            TaskBoardColumn(
                status: .done,
                rows: [done("o", "bil-20", daysAgo: 30, on: "d1"), done("m", "bil-21", daysAgo: 2, on: "d2")]
                    + filler + [done("n", "bil-22", daysAgo: 0.5, on: "d3")]),
            TaskBoardColumn(status: .backlog, rows: [Self.row("b", "bil-30", .backlog, worktree: "c1")]),
        ])
        let drawn = TaskBoardModel.order.flatMap { status in
            board.sections.first { $0.status == status }?.orderedRows ?? []
        }.compactMap(\.worktreeID)
        let order = WorkspaceWorktrees.entries(in: Self.billingSummary, host: "", board: board, fleet: fleet, now: now)
            .compactMap { $0.task?.worktreeID }
        #expect(order == drawn)
        #expect(order == ["c1", "d3", "d2", "d1"])
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

    /// The navigator's sections (ov-92): Tasks holds the board's tasks and
    /// nothing else; Worktrees holds the loose ones, never a task's, which
    /// its task's row names; the orchestrator is its own row, never a
    /// worktree's, and the main checkout it runs in lists every terminal
    /// but it.
    @Test("Each navigator section holds its own rows: worktrees apart from tasks, the orchestrator in neither")
    func theNavigatorsSections() {
        let fleet = Self.fleet()
        for (summary, board) in [(Self.billingSummary, Self.board), (Self.workspaces[0], TaskBoardModel.empty)] {
            let loose = WorkspaceWorktrees.loose(in: summary, host: "", board: board, fleet: fleet)
            let worktrees = BoardWorktreesSection.rows(BoardWorktrees(shown: loose.shown, hidden: loose.hidden))
            let items = Navigator.items(
                orchestrator: true,
                tasks: BoardKeys.rows(board, collapsed: [], reads: BoardReads(floor: .distantPast), filtering: true, now: .now),
                worktrees: worktrees.map(\.id))
            let tasked = Set(WorkspaceWorktrees.taskWorktrees(on: board, host: "", in: fleet).values.map(\.id))
            #expect(items.first == .orchestrator && items.filter { $0 == .orchestrator }.count == 1)
            for item in items {
                switch item {
                case .orchestrator, .unread: break
                case .task(let id): #expect(board.rows.contains { $0.id == id }, "\(id) isn't a task")
                case .worktree(let id): #expect(!tasked.contains(id), "\(id) is a task's, in Worktrees")
                }
            }
            // Tasks, then worktrees: never one among the other.
            let kinds = items.dropFirst().map { if case .task = $0 { 0 } else { 1 } }
            #expect(kinds == kinds.sorted(), "\(items)")
            for worktree in worktrees {
                #expect(!worktree.terminals.contains { $0.isOrchestrator }, "\(worktree.id) lists the orchestrator")
            }
        }
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
        #expect(SidebarDefault.shown(stored: nil, hasHistory: true, collapsedBefore: true) == false)
    }

    /// Read from the defaults themselves (review m3): history is any key
    /// an earlier launch leaves, and a sidebar AppKit saved collapsed stays
    /// collapsed.
    @Test("The sidebar's default is read from what the defaults hold")
    func sidebarDefaultFromDefaults() throws {
        func fresh() throws -> UserDefaults {
            let name = "ov86-\(UUID().uuidString)"
            let defaults = try #require(UserDefaults(suiteName: name))
            defaults.removePersistentDomain(forName: name)
            return defaults
        }
        #expect(SidebarDefault.shown(in: try fresh()) == false)
        let collapsedProjects = try fresh()
        collapsedProjects.set("", forKey: "sidebar.collapsedProjects")
        #expect(SidebarDefault.shown(in: collapsedProjects) == true)
        let collapsed = try fresh()
        collapsed.set("general", forKey: "settings.tab")
        collapsed.set(
            ["0.000000, 0.000000, 320.000000, 1130.000000, YES, NO", "0, 0, 1800, 1130, NO, NO"],
            forKey: "NSSplitView Subview Frames X-1-AppWindow-1, SidebarNavigationSplitView")
        #expect(SidebarDefault.shown(in: collapsed) == false)
        let stored = try fresh()
        stored.set("shown", forKey: SidebarDefault.key)
        #expect(SidebarDefault.shown(in: stored) == true)
    }
}
