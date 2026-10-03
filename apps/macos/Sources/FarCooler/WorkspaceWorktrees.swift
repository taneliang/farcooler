import AgentKit
import Foundation

// Getting around a workspace's worktrees without the sidebar (ov-86).
//
// The board list is the navigator inside a workspace: each task's row names
// its worktree, and a Worktrees section at the bottom holds the ones no task
// has, the main checkout and scratch ones. ⌃⌘↓ and ⌃⌘↑ walk them in that
// order, and the breadcrumb's worktree menu, straight under the workspace,
// lists them the same way: task ones by their task, then the loose ones.
// After a task's crumb it holds only that task's own (ov-185).
//
// Worked out here, as values, so the order, the menu and the section's
// membership are the ones `WorkspaceWorktreesTests` pins.

enum WorkspaceWorktrees {
    typealias Selection = ContentView.Selection

    /// One worktree in the workspace's order: a task's, or a loose one.
    struct Entry: Equatable {
        var worktree: Worktree
        /// The task it's the worktree of, or nil for a loose one.
        var task: TaskRow?
    }

    /// The worktree `row` names: its own, else its agent's, as the task view
    /// draws beneath it (`TaskColumnModel.worktree`). Nil when it has none,
    /// or the fleet doesn't list it.
    static func worktree(of row: TaskRow, host: String, in fleet: Fleet) -> Worktree? {
        let agent = WorkspaceScreen.agent(of: row.id, host: host, in: fleet, chosen: nil)
        guard let id = TaskColumnModel.worktree(of: row, agent: agent) else { return nil }
        return WorkspaceSelection.worktree(host: host, id: id, in: fleet)
    }

    /// The worktrees `board`'s tasks name, keyed by task id: what a task's
    /// row shows beside its key.
    static func taskWorktrees(on board: TaskBoardModel, host: String, in fleet: Fleet) -> [String: Worktree] {
        var out: [String: Worktree] = [:]
        for row in board.rows {
            if let worktree = worktree(of: row, host: host, in: fleet) { out[row.id] = worktree }
        }
        return out
    }

    /// The Worktrees section under `workspace`'s board: the worktrees it
    /// holds that no task on the board names. Main's also holds its
    /// repository's unclaimed ones, which open beside its board. Each is
    /// drawn without the orchestrators seated in it, which only their
    /// conversation shows (the one-place rule). Hidden ones are kept apart,
    /// for the section's collapsed Hidden row.
    static func loose(
        in workspace: WorkspaceSummary, host: String, board: TaskBoardModel, fleet: Fleet
    ) -> (shown: [Worktree], hidden: [Worktree]) {
        let tasked = Set(taskWorktrees(on: board, host: host, in: fleet).values.map(\.id))
        let mine = fleet.worktrees.filter { worktree in
            guard (worktree.host ?? "") == host, !tasked.contains(worktree.id) else { return false }
            if let owner = WorkspaceSelection.owner(of: worktree, in: fleet) { return owner == workspace.id }
            // Unclaimed: beside its repository's Main board.
            return workspace.isMain && workspace.repository != nil && worktree.repositoryID == workspace.repository
        }
        .map { WorkspaceScreen.ownTerminals(of: $0, fleet: fleet) }
        return (mine.filter { !$0.isHidden }, mine.filter(\.isHidden))
    }

    /// The workspace's worktrees in the order the board list draws them:
    /// each task's, section by section, with each section's rows as the
    /// list draws them (Done newest first, `orderedRows`), then the
    /// Worktrees section's, then its Hidden ones, in the runner's order. A
    /// collapsed section, the Done tasks the list cuts and the hidden
    /// worktrees are walked too, in the place they're drawn when shown, so
    /// none of them is out of reach (ov-86 review M2). Each worktree once,
    /// under the first task naming it.
    static func entries(
        in workspace: WorkspaceSummary, host: String, board: TaskBoardModel, fleet: Fleet, now: Date = Date()
    ) -> [Entry] {
        var seen = Set<String>()
        var out: [Entry] = []
        for section in board.sections {
            for row in section.orderedRows {
                guard let worktree = worktree(of: row, host: host, in: fleet), seen.insert(worktree.id).inserted
                else { continue }
                out.append(Entry(worktree: worktree, task: row))
            }
        }
        let loose = loose(in: workspace, host: host, board: board, fleet: fleet)
        for worktree in loose.shown + loose.hidden where seen.insert(worktree.id).inserted {
            out.append(Entry(worktree: worktree, task: nil))
        }
        return out
    }

    /// Where `selection` is in `entries`: the task open, or the worktree
    /// open whole. Nil at the board alone, or anywhere else.
    static func index(of selection: Selection?, in entries: [Entry]) -> Int? {
        switch selection {
        case .workspace(_, _, .task(let id)?):
            return entries.firstIndex { $0.task?.id == id }
        case .workspace(_, _, .worktree(let id, _)?), .looseWorktree(_, let id, _):
            return entries.firstIndex { $0.worktree.id == id }
        default:
            return nil
        }
    }

    /// Where going to `entry` lands: its task, opened beside the board with
    /// the worktree beneath it, or a loose worktree opened whole.
    static func target(of entry: Entry, host: String, workspace: String, in fleet: Fleet) -> Selection {
        if let task = entry.task { return .workspace(host: host, workspace: workspace, focus: .task(task.id)) }
        return ContentView.opening(entry.worktree, terminal: nil, in: fleet)
    }

    /// ⌃⌘↓ (`by` 1) and ⌃⌘↑ (−1): the next or previous worktree from where
    /// `selection` is, wrapping, as ⌘] does through terminals. From the
    /// board alone, the first going down and the last going up. Nil with
    /// none to go to, or only the one already open.
    static func step(
        from selection: Selection?, by: Int, in entries: [Entry], host: String, workspace: String, fleet: Fleet
    ) -> Selection? {
        guard !entries.isEmpty else { return nil }
        let next: Int
        if let at = index(of: selection, in: entries) {
            next = ((at + by) % entries.count + entries.count) % entries.count
            if next == at { return nil }
        } else {
            next = by >= 0 ? 0 : entries.count - 1
        }
        return target(of: entries[next], host: host, workspace: workspace, in: fleet)
    }

    /// One item of the breadcrumb's worktree menu.
    struct MenuItem: Equatable, Identifiable {
        /// A task's key and title, or a loose worktree's name.
        var title: String
        /// A task's worktree's name, under its title.
        var subtitle: String?
        var target: Selection
        /// Where the window is now: checked.
        var current: Bool
        /// The task it's opened from, kept as the way back: set for a
        /// task's own worktrees, as Open Worktree does.
        var trail: Selection? = nil

        /// Where it goes: two worktrees can share a name, never a place.
        var id: Selection { target }
    }

    /// The breadcrumb's worktree menu: the task ones, labelled with their
    /// task, then the loose ones, each a way there.
    static func menu(
        _ entries: [Entry], selection: Selection?, host: String, workspace: String, fleet: Fleet
    ) -> (tasks: [MenuItem], loose: [MenuItem]) {
        let here = index(of: selection, in: entries)
        var tasks: [MenuItem] = []
        var loose: [MenuItem] = []
        for (at, entry) in entries.enumerated() {
            let target = target(of: entry, host: host, workspace: workspace, in: fleet)
            if let task = entry.task {
                tasks.append(
                    MenuItem(
                        title: "\(task.key) \(task.title)", subtitle: entry.worktree.task, target: target,
                        current: at == here))
            } else {
                loose.append(MenuItem(title: entry.worktree.task, subtitle: nil, target: target, current: at == here))
            }
        }
        return (tasks, loose)
    }

    /// The worktrees task `row` works in, its own first: `Task.worktree_id`
    /// (which is also what puts it in a worktree's `open_tasks`, the
    /// runner's lane fallback), then each worktree holding a pane opened for
    /// it (`Terminal.task_id`), live or not, in the runner's order. Never by
    /// a worktree's name: a lane called ov-177 is ov-177's by a person's
    /// habit, which the runner doesn't know, and a guess here would offer
    /// another task's work under this one.
    static func worktrees(of row: TaskRow, host: String, in fleet: Fleet) -> [Worktree] {
        let mine = fleet.worktrees.filter { ($0.host ?? "") == host }
        var out: [Worktree] = []
        if let own = row.worktreeID, !own.isEmpty, let found = mine.first(where: { $0.id == own }) {
            out.append(found)
        }
        for worktree in mine where !out.contains(where: { $0.id == worktree.id }) {
            let lane = worktree.openTaskIDs.contains(row.id)
            let pane = worktree.terminals.contains { !$0.isOrchestrator && $0.taskId == row.id }
            if lane || pane { out.append(worktree) }
        }
        return out
    }

    /// The breadcrumb's worktree segment, at the level it's drawn at.
    struct Segment: Equatable {
        /// What it says after the branch glyph.
        var title: String
        /// Whether it's the level you're at, a worktree opened whole, in
        /// place of that worktree's own crumb.
        var isHere: Bool
        /// The menu's items: task worktrees by their task, then loose ones.
        /// Beneath a task, only that task's own, all in `loose`.
        var tasks: [MenuItem] = []
        var loose: [MenuItem] = []
        /// Beside a task with one worktree: no menu, a way into that one.
        var opens: MenuItem?
        /// Whether the menu is the whole workspace's.
        var isWorkspace = false
    }

    /// The breadcrumb's worktree segment for `place` (ov-185). A level's menu
    /// lists what sits beside it, as a jump bar's does:
    ///
    /// - A worktree opened whole stands in for its own crumb. Straight under
    ///   the workspace, its menu is every worktree in the workspace, the
    ///   order ⌃⌘↑ and ⌃⌘↓ walk. Opened from a task (`trail`), it's that
    ///   task's worktrees.
    /// - After a task's crumb, the task's own worktrees
    ///   (`taskWorktrees`): none is no segment, one is a way into it, and
    ///   several a menu of just those.
    ///
    /// Nil at the board alone. `host` and `workspace` are the board's, for
    /// the workspace's menu.
    static func segment(
        for place: Selection, trail: Selection?, entries: [Entry], taskWorktrees: (_ task: String) -> [Worktree],
        name: (_ host: String, _ worktree: String) -> String?, host: String, workspace: String, fleet: Fleet
    ) -> Segment? {
        let task: (from: Selection, id: String)? = {
            if case .workspace(_, _, .task(let id)?) = place { return (place, id) }
            if case .workspace(_, _, .worktree?) = place, let trail, case .workspace(_, _, .task(let id)?) = trail {
                return (trail, id)
            }
            return nil
        }()
        let here: (host: String, id: String)? = {
            switch place {
            case .workspace(let host, _, .worktree(let id, _)?), .looseWorktree(let host, let id, _): return (host, id)
            default: return nil
            }
        }()
        var items: [MenuItem] = []
        if let task {
            items = taskWorktrees(task.id).map { worktree in
                MenuItem(
                    title: worktree.task, subtitle: nil,
                    target: WorkspaceNavigation.openWorktree(worktree.id, from: task.from).next,
                    current: worktree.id == here?.id, trail: task.from)
            }
        }
        if let here {
            let title = name(here.host, here.id) ?? "Worktree"
            guard let task else {
                let menu = menu(entries, selection: place, host: host, workspace: workspace, fleet: fleet)
                return Segment(title: title, isHere: true, tasks: menu.tasks, loose: menu.loose, isWorkspace: true)
            }
            // Opened from its task, though the task no longer names it.
            if !items.contains(where: \.current) {
                let target = WorkspaceNavigation.openWorktree(here.id, from: task.from).next
                items.insert(MenuItem(title: title, subtitle: nil, target: target, current: true, trail: task.from), at: 0)
            }
            return Segment(title: title, isHere: true, loose: items)
        }
        guard task != nil else { return nil }
        switch items.count {
        case 0: return nil
        case 1: return Segment(title: items[0].title, isHere: false, opens: items[0])
        default: return Segment(title: "Worktrees", isHere: false, loose: items)
        }
    }

    /// The crumbs drawn before the segment: all of them beside a task's
    /// worktree, all but the last when the segment stands for it.
    static func crumbs(_ crumbs: [WorkspaceNavigation.Crumb], isHere: Bool) -> [WorkspaceNavigation.Crumb] {
        isHere ? Array(crumbs.dropLast()) : crumbs
    }
}

/// ⌘1 through ⌘9 (ov-86): the first nine workspaces, in the order the
/// title bar's switcher lists them, which is the sidebar's: each runner's
/// repositories, each repository's workspaces.
enum WorkspaceNumbers {
    /// How many workspaces have a ⌘-number.
    static let count = 9

    /// One workspace, as the switcher lists it.
    struct Place: Equatable {
        var host: String
        var workspace: WorkspaceSummary
        /// Its name: Main for a repository's implicit workspace.
        var name: String
        /// Its ⌘-number, 1 to 9, or nil past the ninth.
        var number: Int?

        var selection: ContentView.Selection { .workspace(host: host, workspace: workspace.id, focus: nil) }
    }

    /// One repository's workspaces, under its name.
    struct Group: Equatable {
        var host: String
        var repository: String
        /// The repository's id, or nil from a CLI too old to send one.
        var repositoryID: String? = nil
        var places: [Place]
    }

    /// Every workspace, grouped by repository, numbered in order.
    static func groups(in fleet: Fleet) -> [Group] {
        var out: [Group] = []
        var number = 0
        for entry in ContentView.sidebarRows(fleet: fleet) {
            switch entry.kind {
            case .repository:
                out.append(Group(host: entry.host, repository: entry.project, repositoryID: entry.repositoryID, places: []))
            case .workspace(let name):
                guard let workspace = entry.workspace, !out.isEmpty else { continue }
                number += 1
                out[out.count - 1].places.append(
                    Place(host: entry.host, workspace: workspace, name: name, number: number <= count ? number : nil))
            default:
                continue
            }
        }
        return out.filter { !$0.places.isEmpty }
    }

    /// Where ⌘`number` goes: the workspace with that number, or nil when
    /// there are fewer.
    static func target(_ number: Int, in groups: [Group]) -> ContentView.Selection? {
        groups.flatMap(\.places).first { $0.number == number }?.selection
    }
}
