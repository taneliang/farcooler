import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The jump bar's menus (ov-192), one sheet per segment kind, each drawn as
/// it opens under its bar, in both appearances, so they can be looked at.
/// Not an assertion beyond "it drew": a rendering, written where
/// `FARCOOLER_GLANCE_OUT` says.
@MainActor
struct JumpBarSpecimenTests {
    private typealias Selection = ContentView.Selection
    private static let ws = "billing"

    private static func task(_ id: String) -> Selection { .workspace(host: "", workspace: ws, focus: .task(id)) }
    private static func whole(_ id: String, _ terminal: String? = nil) -> Selection {
        .workspace(host: "", workspace: ws, focus: .worktree(id, terminal: terminal))
    }

    private static func terminal(_ id: String, _ preset: String, state: String = "running") -> Terminal {
        Terminal(id: id, short: id, title: id, preset: preset, state: state, epoch: 0)
    }

    private static func row(_ id: String, _ key: String, _ title: String, _ status: TaskStatus, _ age: TimeInterval = 0)
        -> TaskRow
    {
        TaskRow(id: id, key: key, title: title, status: status, statusSince: Date(timeIntervalSince1970: 1_000_000 + age))
    }

    private static let main = WorkspaceNavigation.Crumb(title: "Main", target: .workspace(host: "", workspace: ws, focus: nil))

    private static func taskMenu(_ place: Selection, tab: TaskTab?) -> JumpMenu {
        let board = TaskBoardModel(columns: [
            TaskBoardColumn(status: .needsDecision, rows: [row("t54", "lo-54", "Run the evals with Jev deciding", .needsDecision)]),
            TaskBoardColumn(status: .backlog, rows: [
                row("t37", "lo-37", "M20: drop legacy tables", .backlog),
                row("t39", "lo-39", "Remove the 12 legacy chat tool aliases", .backlog),
            ]),
            TaskBoardColumn(status: .inProgress, rows: [
                row("t3", "lo-3", "Coordinator working agreement", .inProgress),
                row("t4", "lo-4", "Prod log watch: adaptive sweeps", .inProgress),
            ]),
            TaskBoardColumn(status: .done, rows: (1...8).map { row("d\($0)", "lo-\(60 + $0)", "Shipped thing \($0)", .done, Double($0)) }),
        ])
        let statuses: [String: Status] = ["t54": .blocked, "t4": .working, "t3": .idle]
        return JumpMenus.workspaceLevel(
            host: "", workspace: ws, place: place, hasOrchestrator: true, orchestrator: .done, board: board,
            taskStatus: { statuses[$0.id] }, taskWorktree: { $0.id == "t4" ? "log-watch" : nil },
            loose: [Worktree(
                id: "lovubot", short: "lovubot", task: "lovubot (main checkout)", branch: "main", repository: "lovubot",
                host: "", path: "/tmp/lovubot", state: "active", terminals: [], repositoryID: "r", workspace: ws)],
            worktreeStatus: { _ in .lost }, opening: { whole($0.id) }, tab: tab)
    }

    private struct Sheet {
        var name: String
        var crumbs: [WorkspaceNavigation.Crumb]
        var worktrees: WorktreeCrumb?
        var menu: JumpMenu
        var state: JumpBarFocus
    }

    private static var sheets: [Sheet] {
        let groups = [
            WorkspaceNumbers.Group(host: "", repository: "lovubot", places: [
                .init(host: "", workspace: WorkspaceSummary(id: ws, name: "Main", taskPrefix: "lo", isMain: true, ordinal: 0, repository: "r"), name: "Main", number: 1),
                .init(host: "", workspace: WorkspaceSummary(id: "w2", name: "Evals", taskPrefix: "ev", isMain: false, ordinal: 1, repository: "r"), name: "Evals", number: 2),
            ]),
            WorkspaceNumbers.Group(host: "mac-mini", repository: "overnight", places: [
                .init(host: "mac-mini", workspace: WorkspaceSummary(id: "w3", name: "Main", taskPrefix: "ov", isMain: true, ordinal: 0, repository: "o"), name: "Main", number: 3),
            ]),
        ]
        let workspaces = JumpMenus.workspaces(groups, current: ("", ws), showsHosts: true, waiting: { $0.host == "mac-mini" ? 3 : ($0.workspace.id == ws ? 1 : 0) })
        let taskCrumbs = [main, WorkspaceNavigation.Crumb(title: "lo-4 Prod log watch: adaptive sweeps", target: nil)]
        let taskSeg = taskMenu(task("t4"), tab: .agent)

        let here = Worktree(
            id: "lovubot", short: "lovubot", task: "lovubot", branch: "main", repository: "lovubot", host: "",
            path: "/tmp/lovubot", state: "active",
            terminals: [terminal("s1", "zsh"), terminal("c1", "claude"), terminal("l1", "zsh", state: "LOST"), terminal("l2", "zsh", state: "LOST")],
            repositoryID: "r", workspace: ws)
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [here], branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [groups[0].places[0].workspace]
        let siblings = [
            WorkspaceWorktrees.MenuItem(title: "lo-4 Prod log watch", subtitle: "log-watch", target: whole("lw"), current: false),
        ]
        let loose = [WorkspaceWorktrees.MenuItem(title: "lovubot", subtitle: nil, target: whole("lovubot"), current: true)]
        let worktree = WorktreeCrumb(
            title: "lovubot", isHere: true, tasks: siblings, loose: loose, worktree: "lovubot",
            actions: [.showChanges, .newTerminal, .move(workspace: "w2", name: "Evals"), .hide],
            children: JumpMenus.terminals(of: here, selection: whole("lovubot", "s1"), fleet: fleet))

        // ov-185: after a task's crumb, only that task's worktrees.
        let own = [
            WorkspaceWorktrees.MenuItem(title: "log-watch", subtitle: nil, target: whole("lw"), current: false, trail: task("t4")),
            WorkspaceWorktrees.MenuItem(title: "log-watch-2", subtitle: nil, target: whole("lw2"), current: false, trail: task("t4")),
        ]
        let taskWorktrees = WorktreeCrumb(title: "Worktrees", isHere: false, tasks: [], loose: own)
        let single = WorktreeCrumb(title: "log-watch", isHere: false, tasks: [], loose: [], opens: own[0])

        func open(_ menu: JumpMenu, at segment: Int, query: String = "") -> JumpBarFocus {
            var state = JumpBarFocus(segment: segment, open: true, query: query, highlighted: (menu.current ?? menu.items.first)?.id)
            if !query.isEmpty { state.highlighted = menu.filtered(query).items.first?.id }
            return state
        }
        return [
            Sheet(name: "workspace", crumbs: taskCrumbs, worktrees: nil, menu: workspaces, state: open(workspaces, at: 0)),
            Sheet(name: "task", crumbs: taskCrumbs, worktrees: nil, menu: taskSeg, state: open(taskSeg, at: 1)),
            Sheet(name: "task-filtered", crumbs: taskCrumbs, worktrees: nil, menu: taskSeg, state: open(taskSeg, at: 1, query: "lo-3")),
            Sheet(name: "history", crumbs: [main, .init(title: "Done", target: nil)], worktrees: nil,
                  menu: taskMenu(.workspace(host: "", workspace: ws, focus: .history(.done)), tab: nil),
                  state: open(taskMenu(.workspace(host: "", workspace: ws, focus: .history(.done)), tab: nil), at: 1)),
            Sheet(name: "worktree", crumbs: [main], worktrees: worktree, menu: worktree.jumpMenu, state: open(worktree.jumpMenu, at: 1)),
            Sheet(name: "task-worktrees-ov185", crumbs: taskCrumbs, worktrees: taskWorktrees, menu: taskWorktrees.jumpMenu,
                  state: open(taskWorktrees.jumpMenu, at: 2)),
            Sheet(name: "task-one-worktree-ov185", crumbs: taskCrumbs, worktrees: single, menu: JumpMenu([]), state: .away),
        ]
    }

    @Test("Write the jump bar menu sheets")
    func writeSheets() throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for sheet in Self.sheets {
            for dark in [false, true] {
                let view = VStack(alignment: .leading, spacing: 0) {
                    DrillBreadcrumb(crumbs: sheet.crumbs, worktrees: sheet.worktrees, onGo: { _ in }, onClose: {})
                        .frame(width: 720)
                    if sheet.state.open {
                        JumpMenuView(menu: sheet.menu, state: sheet.state, onKey: { _ in true }, onPick: { _ in })
                            .background(.regularMaterial)
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
                            .padding(.leading, 120)
                            .padding(.top, 6)
                    }
                }
                .padding(.bottom, 16)
                .frame(width: 720, height: sheet.state.open ? 520 : 60, alignment: .topLeading)
                .background(dark ? Color(white: 0.12) : Color.white)
                let host = NSHostingView(rootView: view)
                host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                host.frame = CGRect(origin: .zero, size: host.fittingSize)
                host.layoutSubtreeIfNeeded()
                let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                host.cacheDisplay(in: host.bounds, to: rep)
                let png = try #require(rep.representation(using: .png, properties: [:]))
                try png.write(to: directory.appendingPathComponent("jumpbar-\(sheet.name)-\(dark ? "dark" : "light").png"))
            }
        }
    }
}
