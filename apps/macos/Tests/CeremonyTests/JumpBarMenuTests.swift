import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// What each jump bar segment's menu holds (ov-192): the segment's siblings,
/// where you are checked, and its children where they help; and the filter,
/// which keeps the menu's order.
@MainActor
struct JumpBarMenuTests {
    private typealias Selection = ContentView.Selection

    private static let billing = "0198f2c0-0000-7000-8000-0000000000dd"
    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"

    private static func workspace(_ id: String, _ name: String) -> WorkspaceSummary {
        WorkspaceSummary(id: id, name: name, taskPrefix: "bil", isMain: false, ordinal: 1, repository: repo)
    }

    private static func terminal(_ id: String, state: String = "running", taskId: String? = nil) -> Terminal {
        var t = Terminal(id: id, short: id, title: id, preset: "zsh", state: state, epoch: 0)
        t.taskId = taskId
        return t
    }

    private static func worktree(_ id: String, terminals: [Terminal] = []) -> Worktree {
        Worktree(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: "shop", host: "", path: "/tmp/\(id)",
            state: "active", terminals: terminals, repositoryID: repo, workspace: billing)
    }

    private static func row(_ id: String, _ status: TaskStatus, finished: TimeInterval = 0) -> TaskRow {
        TaskRow(
            id: id, key: "bil-\(id)", title: "Task \(id)", status: status,
            statusSince: Date(timeIntervalSince1970: 1_000_000 + finished))
    }

    private static func task(_ id: String) -> Selection { .workspace(host: "", workspace: billing, focus: .task(id)) }

    // MARK: - The workspace segment

    @Test("The workspace segment lists every workspace by repository, the runner named, this one checked")
    func workspaceSegment() {
        let groups = [
            WorkspaceNumbers.Group(
                host: "", repository: "shop",
                places: [
                    .init(host: "", workspace: Self.workspace(Self.billing, "Billing"), name: "Billing", number: 1),
                    .init(host: "", workspace: Self.workspace("w2", "Search"), name: "Search", number: 2),
                ]),
            WorkspaceNumbers.Group(
                host: "mini", repository: "infra",
                places: [.init(host: "mini", workspace: Self.workspace("w3", "Main"), name: "Main", number: 3)]),
        ]
        let menu = JumpMenus.workspaces(
            groups, current: ("", Self.billing), showsHosts: true, waiting: { $0.workspace.id == "w3" ? 2 : 0 })
        #expect(menu.sections.map(\.title) == ["shop", "infra · mini"])
        #expect(menu.items.map(\.title) == ["Billing", "Search", "Main"])
        #expect(menu.items.map(\.current) == [true, false, false])
        #expect(menu.items.map(\.waiting) == [0, 0, 2])
        #expect(menu.items[2].needsYou && !menu.items[1].needsYou)
        #expect(menu.items[1].target == .go(.workspace(host: "", workspace: "w2", focus: nil)))
        // One runner: no runner in the header.
        let one = JumpMenus.workspaces(groups, current: nil, showsHosts: false, waiting: { _ in 0 })
        #expect(one.sections.map(\.title) == ["shop", "infra"])
    }

    // MARK: - The task segment

    private static func taskMenu(place: Selection, tab: TaskTab? = .agent) -> JumpMenu {
        let done = (1...7).map { row("d\($0)", .done, finished: TimeInterval($0)) }
        let board = TaskBoardModel(columns: [
            TaskBoardColumn(status: .backlog, rows: [row("b1", .backlog)]),
            TaskBoardColumn(status: .inProgress, rows: [row("p1", .inProgress), row("p2", .inProgress)]),
            TaskBoardColumn(status: .needsDecision, rows: [row("n1", .needsDecision)]),
            TaskBoardColumn(status: .done, rows: done),
        ])
        return JumpMenus.workspaceLevel(
            host: "", workspace: billing, place: place, hasOrchestrator: true, orchestrator: .working, board: board,
            taskStatus: { $0.id == "n1" ? .blocked : nil }, taskWorktree: { $0.id == "p1" ? "pdf" : nil },
            loose: [worktree("scratch")], worktreeStatus: { _ in nil },
            opening: { .workspace(host: "", workspace: billing, focus: .worktree($0.id, terminal: nil)) }, tab: tab)
    }

    @Test("A task's segment: its tabs, the orchestrator, tasks by status in the board's order, the loose worktrees")
    func taskSegment() {
        let menu = Self.taskMenu(place: Self.task("p1"))
        #expect(
            menu.sections.map(\.title) == [
                "bil-p1", "", "Needs Decision", "Backlog", "In Progress", "Done", "Worktrees",
            ])
        #expect(menu.sections[0].items.map(\.title) == ["Overview", "Agent", "Changes"])
        #expect(menu.sections[0].items.map(\.current) == [false, true, false])
        #expect(menu.sections[0].items[2].target == .tab(Self.task("p1"), .changes))
        #expect(menu.current?.id == "tab|agent")
        let p1 = menu.items.first { $0.id == "task|p1" }
        #expect(p1?.current == true && p1?.subtitle == "pdf")
        #expect(menu.items.first { $0.id == "orchestrator" }?.status == .working)
        #expect(menu.items.first { $0.id == "task|n1" }?.needsYou == true)
        #expect(menu.items.last?.target == .go(.workspace(host: "", workspace: Self.billing, focus: .worktree("scratch", terminal: nil))))
    }

    @Test("Done lists its newest five, then the way to its History page")
    func doneIsCut() {
        let done = Self.taskMenu(place: Self.task("p1")).sections.first { $0.title == "Done" }
        #expect(done?.items.map(\.id) == ["task|d7", "task|d6", "task|d5", "task|d4", "task|d3", "history|done"])
        #expect(done?.items.last?.title == "Show All Done")
        #expect(done?.more.map(\.id) == ["task|d2", "task|d1"])
        #expect(done?.items.last?.target == .go(.workspace(host: "", workspace: Self.billing, focus: .history(.done))))
        // On the History page, its item is the one checked, and no tabs.
        let history = Self.taskMenu(
            place: .workspace(host: "", workspace: Self.billing, focus: .history(.done)), tab: nil)
        #expect(history.current?.id == "history|done")
        #expect(history.sections.first?.items.map(\.title) == ["Orchestrator"])
        // Untitled: the row says it.
        #expect(history.sections.first?.title == "")
    }

    // MARK: - The worktree segment

    @Test("A worktree's segment: its siblings, its terminals, the lost ones apart going to their page, then its own items")
    func worktreeSegment() {
        let here = Self.worktree("lovubot", terminals: [Self.terminal("t1"), Self.terminal("t2", state: "LOST")])
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [here], branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [Self.workspace(Self.billing, "Billing")]
        let selection = Selection.workspace(host: "", workspace: Self.billing, focus: .worktree("lovubot", terminal: "t1"))
        let sibling = WorkspaceWorktrees.MenuItem(
            title: "lovubot", subtitle: nil, target: WorkspaceSelection.place(selection), current: true)
        let fromTask = WorkspaceWorktrees.MenuItem(
            title: "pdf", subtitle: nil,
            target: .workspace(host: "", workspace: Self.billing, focus: .worktree("pdf", terminal: nil)),
            current: false, trail: Self.task("p1"))
        let menu = JumpMenus.worktree(
            siblings: (tasks: [], loose: [sibling, fromTask]),
            children: JumpMenus.terminals(of: here, selection: selection, fleet: fleet),
            actions: [.open, .move(workspace: "w2", name: "Search"), .hide], named: "lovubot")
        #expect(menu.sections.map(\.title) == ["Worktrees", "Terminals", "Lost", "lovubot"])
        #expect(menu.items[1].target == .open(fromTask.target, from: Self.task("p1")))
        let terminals = menu.sections[1].items
        #expect(terminals.map(\.id) == ["term|t1"] && terminals[0].current)
        let lost = menu.sections[2].items
        #expect(lost[0].status == .lost && lost[0].needsYou)
        // Its page (ov-191), then the answers that page offers, in
        // `LostPane`'s own words and order, each for that pane.
        #expect(lost[0].target == .go(.workspace(host: "", workspace: Self.billing, focus: .worktree("lovubot", terminal: "t2"))))
        let pane = PaneRef(host: "", worktree: "lovubot", terminal: "t2")
        #expect(lost.dropFirst().map(\.title) == LostPane.actions(for: .lost).map(\.title))
        #expect(lost.dropFirst().map(\.target) == LostPane.actions(for: .lost).map { .lost(pane, $0) })
        #expect(lost.dropFirst().allSatisfy { $0.subtitle == lost[0].title })
        #expect(menu.sections[3].items.map(\.title) == ["Open", "Move to Search", "Hide"])
        #expect(menu.sections[3].items[1].target == .worktree(.move(workspace: "w2", name: "Search")))
    }

    @Test("Beside a task with one worktree, the segment's menu is that one, for the keyboard")
    func oneWorktree() {
        let item = WorkspaceWorktrees.MenuItem(
            title: "pdf", subtitle: nil,
            target: .workspace(host: "", workspace: Self.billing, focus: .worktree("pdf", terminal: nil)),
            current: false, trail: Self.task("p1"))
        let crumb = WorktreeCrumb(title: "pdf", isHere: false, tasks: [], loose: [], opens: item)
        #expect(crumb.jumpMenu.items.map(\.title) == ["pdf"])
        let segments = DrillBreadcrumb.segmentMenus(crumbs: 2, menus: [Self.taskMenu(place: Self.task("p1"))], worktrees: crumb)
        #expect(segments.count == 3)
        #expect(segments[1].isEmpty)
        #expect(segments[2].items.map(\.title) == ["pdf"])
    }

    // MARK: - Type to filter

    @Test("Filtering keeps the menu's order, matches subtitles, and drops emptied sections")
    func filtering() {
        let menu = Self.taskMenu(place: Self.task("p1"))
        let p = menu.filtered("bil p")
        #expect(p.items.map(\.id) == ["task|p1", "task|p2"])
        #expect(p.sections.map(\.title) == ["In Progress"])
        // By a task's worktree, its subtitle.
        #expect(menu.filtered("pdf").items.map(\.id) == ["task|p1"])
        // Done's older tasks are found too, before the way to History.
        #expect(menu.filtered("bil-d1").items.map(\.id) == ["task|d1", "history|done"])
        #expect(menu.filtered("  ") == menu)
        #expect(menu.filtered("zzzz").isEmpty)
        #expect(JumpMenu.noMatches("zz") == "No Matches for “zz”")
    }

    /// lo-37, listed first, scores lower than lo-3 for "lo-3" (its title is
    /// longer): a filter that ranked would swap them.
    private static let keyed = JumpMenu([
        JumpSection(title: "Backlog", items: [
            JumpItem(id: "37", title: "lo-37 Remove the twelve legacy chat tool aliases from the gateway", key: "lo-37",
                     target: .go(task("37"))),
            JumpItem(id: "3", title: "lo-3 Coordinator agreement", key: "lo-3", target: .go(task("3"))),
            JumpItem(id: "30", title: "lo-30 Rotate keys", key: "lo-30", target: .go(task("30"))),
        ])
    ])

    @Test("A filter keeps the menu's order even where a later row scores higher")
    func orderNotScore() {
        #expect(Fuzzy.score(Self.keyed.items[1].title, "lo-3")! > Fuzzy.score(Self.keyed.items[0].title, "lo-3")!)
        #expect(Self.keyed.filtered("lo-3").items.map(\.id) == ["37", "3", "30"])
    }

    @Test("The best match: the key named, then the shortest key begun, then a title word, then the score")
    func bestMatch() {
        #expect(Self.keyed.best("lo-3")?.id == "3")
        #expect(Self.keyed.best("LO-37")?.id == "37")
        #expect(Self.keyed.best("lo-")?.id == "3")
        #expect(Self.keyed.best("rotate")?.id == "30")
        #expect(Self.keyed.best("coord")?.id == "3")
        #expect(Self.keyed.best("zzz") == nil)
    }

    @Test("A menu is as wide as its widest row, within bounds")
    func width() {
        let narrow = JumpMenu([JumpSection(title: "", items: [JumpItem(id: "a", title: "a", target: .go(Self.task("a")))])])
        #expect(JumpMenuView.width(for: narrow) == JumpMenuView.minWidth)
        #expect(JumpMenuView.width(for: Self.keyed) == JumpMenuView.maxWidth)
        let middle = JumpMenu([JumpSection(title: "", items: [JumpItem(id: "a", title: "lo-3 Coordinator agreement", subtitle: "pdf", target: .go(Self.task("a")))])])
        let w = JumpMenuView.width(for: middle)
        #expect(w > JumpMenuView.minWidth && w < JumpMenuView.maxWidth)
    }

    // MARK: - Wiring to the window

    @Test("Only the crumb you're at gets the task's tabs; an ancestor task's menu checks the task")
    func tabsOnlyOnTheLastCrumb() {
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [], branchPrefix: nil)
        let summary = Self.workspace(Self.billing, "Billing")
        fleet.runnerWorkspaces[""] = [summary]
        let board = TaskBoardModel(columns: [TaskBoardColumn(status: .inProgress, rows: [Self.row("p1", .inProgress)])])
        func menus(_ crumbs: [WorkspaceNavigation.Crumb], place: Selection) -> [JumpMenu] {
            JumpMenus.crumbs(
                crumbs, place: place, host: "", workspace: summary, board: board, fleet: fleet, groups: [],
                showsHosts: false, waiting: { _ in 0 }, tab: .changes, chosen: { _ in nil })
        }
        let top = WorkspaceNavigation.Crumb(title: "Billing", target: .workspace(host: "", workspace: Self.billing, focus: nil))
        let atTask = menus([top, .init(title: "bil-p1", target: nil)], place: Self.task("p1"))
        #expect(atTask.count == 2)
        #expect(atTask[1].current?.target == .tab(Self.task("p1"), .changes))
        // A worktree opened from the task: the task's crumb is an ancestor.
        let worktree = Selection.workspace(host: "", workspace: Self.billing, focus: .worktree("pdf", terminal: nil))
        let below = menus([top, .init(title: "bil-p1", target: Self.task("p1"))], place: worktree)
        #expect(!below[1].items.contains { $0.id.hasPrefix("tab|") })
        #expect(below[1].current?.id == "task|p1")
    }

    @Test("A chosen item is routed: worktree items performed, a task's worktree keeping its trail, the rest the window's")
    func routing() {
        let wt = Selection.workspace(host: "", workspace: Self.billing, focus: .worktree("pdf", terminal: nil))
        #expect(DrillBreadcrumb.routed(.worktree(.hide)) == .perform(.hide))
        #expect(
            DrillBreadcrumb.routed(.open(wt, from: Self.task("p1")))
                == .open(WorkspaceWorktrees.MenuItem(title: "", subtitle: nil, target: wt, current: false, trail: Self.task("p1"))))
        #expect(DrillBreadcrumb.routed(.go(wt)) == .jump(.go(wt)))
        // What the window does with each: a place, and a tab chosen with it.
        #expect(JumpTarget.tab(Self.task("p1"), .changes).place == Self.task("p1"))
        #expect(JumpTarget.tab(Self.task("p1"), .changes).taskTab! == (task: "p1", tab: .changes))
        #expect(JumpTarget.go(wt).taskTab == nil)
        #expect(JumpTarget.worktree(.hide).place == nil)
        let pane = PaneRef(host: "", worktree: "pdf", terminal: "t2")
        #expect(DrillBreadcrumb.routed(.lost(pane, .dismiss)) == .jump(.lost(pane, .dismiss)))
        #expect(JumpTarget.lost(pane, .restart).place == nil)
        // The window runs it through ov-191's one mapping.
        #expect(TerminalAction(LostPane.Action.dismiss) == .dismissLost)
    }
}
