import AgentKit
import Foundation

// What each jump bar segment's menu holds (ov-192), as Xcode's: the
// segment's siblings, the one you're at checked, and where it helps, its
// children in a section of their own. Worked out here, as values, so
// `JumpBarMenuTests` pins each segment's contents and the filter.
//
// The jump bar is local and by place: what's beside where you are, in the
// board's order. The palette (⌘P) is global and by name, ranked. So nothing
// here ranks, creates or reaches past the workspace but through its first
// segment.

/// Where a jump bar item goes. Already resolved, in this window's own ids:
/// a `Destination` (ov-182) is the portable place, resolved later, which
/// every item here already is.
enum JumpTarget: Hashable {
    case go(ContentView.Selection)
    /// A task's worktree, opened with the task kept as the way back, as its
    /// Open Worktree does (ov-185's `MenuItem.trail`).
    case open(ContentView.Selection, from: ContentView.Selection)
    /// A task's tab, with the task open.
    case tab(ContentView.Selection, TaskTab)
    /// One of the worktree's own menu items, as its row offers them.
    case worktree(WorktreeMenu.Item)
    /// A lost terminal's Restart or Dismiss, as its page and its row offer
    /// them (ov-191's `LostPane`).
    case lost(PaneRef, LostPane.Action)

    /// Where it takes the window, or nil for a worktree's own item.
    var place: ContentView.Selection? {
        switch self {
        case .go(let next), .open(let next, _), .tab(let next, _): next
        case .worktree, .lost: nil
        }
    }

    /// The task and the tab it shows, for a tab.
    var taskTab: (task: String, tab: TaskTab)? {
        if case .tab(.workspace(_, _, .task(let id)?), let tab) = self { return (id, tab) }
        return nil
    }
}

/// One row of a jump bar menu.
struct JumpItem: Identifiable, Equatable {
    var id: String
    var title: String
    /// Quiet, after the title: a task's worktree, a terminal's program.
    var subtitle: String?
    /// A task's key, which a typed query matches first.
    var key: String?
    /// The app's own ring, or nil for a row that's no terminal's or task's.
    var status: Status?
    /// The SF Symbol the sidebar draws for the same kind of row
    /// (`OneTreeGlyph`), before the title (ov-328); nil for a row with no
    /// sidebar row to match: a workspace, a tab, a way to History, an action.
    var glyph: String?
    /// A workspace's waiting count, as the switcher badges it.
    var waiting = 0
    /// Where the window is: checked.
    var current = false
    var target: JumpTarget

    /// Whether it wants you: the one thing drawn in color.
    var needsYou: Bool { (status?.wantsAttention ?? false) || waiting > 0 }
}

struct JumpSection: Identifiable, Equatable {
    /// Its header, or empty for none: the menu's first group, as Xcode's.
    var title: String
    var items: [JumpItem]
    /// Rows not listed until a filter finds them: Done's and Canceled's
    /// older tasks, before the way to their History page.
    var more: [JumpItem] = []
    var id: String { title }
}

/// A segment's menu: its sections, in order.
struct JumpMenu: Equatable {
    var sections: [JumpSection]

    init(_ sections: [JumpSection]) { self.sections = sections.filter { !$0.items.isEmpty } }

    var items: [JumpItem] { sections.flatMap(\.items) }
    var isEmpty: Bool { sections.isEmpty }
    var current: JumpItem? { items.first(where: \.current) }

    /// Type-to-filter: the items whose title or subtitle has the query as a
    /// subsequence (the palette's `Fuzzy`), each section kept while it has
    /// one, its unlisted rows (`more`) searched too, before its last item,
    /// the way to History, which stays while any of them matches.
    /// The order stays the menu's: a map, not a ranking.
    func filtered(_ query: String) -> JumpMenu {
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { return self }
        return JumpMenu(
            sections.map { section in
                guard !section.more.isEmpty, let last = section.items.last else {
                    return JumpSection(title: section.title, items: section.items.filter { Self.matches($0, query) })
                }
                // The way to History stays at the end while any task matches.
                let found = (section.items.dropLast() + section.more).filter { Self.matches($0, query) }
                return JumpSection(title: section.title, items: found.isEmpty ? [] : found + [last])
            })
    }

    static func matches(_ item: JumpItem, _ query: String) -> Bool {
        Fuzzy.score(item.title, query) != nil || item.subtitle.map { Fuzzy.score($0, query) != nil } == true
    }

    /// The item a query means, among those it shows: a key it names
    /// exactly, else a key it begins, the shortest, else a title word it
    /// begins, else the best `Fuzzy` score; the earliest of equals.
    func best(_ query: String) -> JumpItem? {
        let shown = filtered(query).items
        var best: (item: JumpItem, rank: Int)?
        for item in shown {
            let rank = Self.rank(item, query)
            if best == nil || rank > best!.rank { best = (item, rank) }
        }
        return best?.item
    }

    static func rank(_ item: JumpItem, _ query: String) -> Int {
        let q = query.lowercased().trimmingCharacters(in: .whitespaces)
        let fuzzy = Fuzzy.score(item.title, q) ?? item.subtitle.flatMap { Fuzzy.score($0, q) } ?? -1000
        if let key = item.key?.lowercased() {
            if key == q { return 100_000 }
            if key.hasPrefix(q) { return 50_000 - key.count }
        }
        let words = item.title.lowercased().split(whereSeparator: { $0.isWhitespace })
        if words.contains(where: { $0.hasPrefix(q) }) { return 10_000 + fuzzy }
        return fuzzy
    }

    /// What a filter with nothing left says.
    static func noMatches(_ query: String) -> String { "No Matches for “\(query)”" }
}

enum JumpMenus {
    typealias Selection = ContentView.Selection

    /// How many finished tasks a status's section lists before the way to
    /// its History page.
    static let finishedLimit = 5

    /// The workspace segment: every workspace, grouped by repository as the
    /// title bar's switcher groups them, the runner beside the repository
    /// when there's more than one (`showsHosts`); each with its waiting
    /// count.
    static func workspaces(
        _ groups: [WorkspaceNumbers.Group], current: (host: String, workspace: String)?, showsHosts: Bool,
        waiting: (WorkspaceNumbers.Place) -> Int
    ) -> JumpMenu {
        JumpMenu(
            groups.map { group in
                let title = showsHosts && !group.host.isEmpty ? "\(group.repository) · \(group.host)" : group.repository
                return JumpSection(
                    title: title,
                    items: group.places.map { place in
                        JumpItem(
                            id: "ws|\(place.host)|\(place.workspace.id)", title: place.name, waiting: waiting(place),
                            current: current.map { $0.host == place.host && $0.workspace == place.workspace.id } ?? false,
                            target: .go(place.selection))
                    })
            })
    }

    /// The segment under the workspace, a task or a History page: the
    /// workspace's orchestrator, its tasks by status in the board's order
    /// (Done and Canceled cut to the newest, then the way to History), and
    /// its loose worktrees. A task's own tabs come first, as its children.
    static func workspaceLevel(
        host: String, workspace: String, place: Selection, hasOrchestrator: Bool, orchestrator: Status?,
        board: TaskBoardModel,
        taskStatus: (TaskRow) -> Status?, taskWorktree: (TaskRow) -> String?, loose: [Worktree],
        worktreeStatus: (Worktree) -> Status?, opening: (Worktree) -> Selection, tab: TaskTab?
    ) -> JumpMenu {
        var sections: [JumpSection] = []
        let open: (ContentView.Focus) -> Selection = { .workspace(host: host, workspace: workspace, focus: $0) }
        if case .workspace(_, _, .task(let id)?) = place, let tab, let row = board.rows.first(where: { $0.id == id }) {
            sections.append(
                JumpSection(
                    title: row.key,
                    items: TaskTab.allCases.map { each in
                        JumpItem(
                            id: "tab|\(each.rawValue)", title: each.title, current: each == tab,
                            target: .tab(open(.task(id)), each))
                    }))
        }
        if hasOrchestrator {
            sections.append(
                JumpSection(
                    title: "",
                    items: [
                        JumpItem(
                            id: "orchestrator", title: "Orchestrator", status: orchestrator, glyph: OneTreeGlyph.orchestrator,
                            target: .go(.workspace(host: host, workspace: workspace, focus: nil)))
                    ]))
        }
        for status in TaskBoardModel.order {
            guard let column = board.columns.first(where: { $0.status == status }) else { continue }
            let rows = column.orderedRows
            let shown = status.isFinished ? Array(rows.prefix(finishedLimit)) : rows
            func item(_ row: TaskRow) -> JumpItem {
                JumpItem(
                    id: "task|\(row.id)", title: "\(row.key) \(row.title)", subtitle: taskWorktree(row), key: row.key,
                    status: taskStatus(row), glyph: OneTreeGlyph.task(row.status), current: Self.isTask(row.id, place),
                    target: .go(open(.task(row.id))))
            }
            var items = shown.map(item)
            var more: [JumpItem] = []
            // The way to the whole status, while some aren't listed, or
            // while it's where you are. The rest are found by a filter.
            let history = open(.history(status))
            if status.isFinished, rows.count > shown.count || place == history {
                items.append(
                    JumpItem(
                        id: "history|\(status.rawValue)", title: "Show All \(status.title)", current: place == history,
                        target: .go(history)))
                more = rows.dropFirst(shown.count).map(item)
            }
            sections.append(JumpSection(title: status.title, items: items, more: more))
        }
        sections.append(
            JumpSection(
                title: "Worktrees",
                items: loose.map { worktree in
                    JumpItem(
                        id: "wt|\(worktree.id)", title: worktree.task, status: worktreeStatus(worktree),
                        glyph: OneTreeGlyph.worktree,
                        target: .go(opening(worktree)))
                }))
        return JumpMenu(sections)
    }

    private static func isTask(_ id: String, _ place: Selection) -> Bool {
        if case .workspace(_, _, .task(id)?) = place { return true }
        return false
    }

    /// The worktree segment: its siblings by ov-185's rule
    /// (`WorkspaceWorktrees.segment`), then its children, the terminals of
    /// a worktree you're in (`terminals`), then the worktree's own menu's
    /// items under its name (`named`).
    static func worktree(
        siblings: (tasks: [WorkspaceWorktrees.MenuItem], loose: [WorkspaceWorktrees.MenuItem]),
        children: [JumpSection], actions: [WorktreeMenu.Item], named: String?
    ) -> JumpMenu {
        func item(_ m: WorkspaceWorktrees.MenuItem) -> JumpItem {
            JumpItem(
                id: "sib|\(String(describing: m.target))", title: m.title, subtitle: m.subtitle,
                glyph: OneTreeGlyph.worktree, current: m.current,
                target: m.trail.map { .open(m.target, from: $0) } ?? .go(m.target))
        }
        var sections = [
            JumpSection(title: "Tasks", items: siblings.tasks.map(item)),
            JumpSection(title: "Worktrees", items: siblings.loose.map(item)),
        ]
        sections += children
        if let named {
            sections.append(
                JumpSection(
                    title: named,
                    items: actions.map { action in
                        let title: String
                        if case .move(_, let name) = action { title = "Move to \(name)" } else { title = action.title }
                        return JumpItem(id: "act|\(String(describing: action))", title: title, target: .worktree(action))
                    }))
        }
        return JumpMenu(sections)
    }

    /// A worktree's terminals, for its segment's children: the live ones,
    /// then the lost ones apart. A lost one opens its page (ov-191), and
    /// under it are the answers its page offers, by `LostPane`'s own list
    /// and titles, Dismiss only where the runner accepts it. Not the
    /// orchestrators seated in it, which only their conversation shows.
    static func terminals(of worktree: Worktree, selection: Selection?, fleet: Fleet) -> [JumpSection] {
        let host = worktree.host ?? ""
        let own = WorkspaceScreen.ownTerminals(of: worktree, fleet: fleet).terminals.filter { !$0.isOrchestrator }
        let named = WorkspaceScreen.namedTerminal(selection).map {
            PaneRef(host: $0.host, worktree: $0.worktree, terminal: $0.terminal)
        }
        func pane(_ t: Terminal) -> PaneRef { PaneRef(host: host, worktree: worktree.id, terminal: t.id) }
        func item(_ t: Terminal) -> JumpItem {
            JumpItem(
                id: "term|\(t.id)", title: worktree.name(of: t), subtitle: Terminal.name(of: t.preset), status: t.status,
                glyph: OneTreeGlyph.terminal(isAgent: ContentView.isAgent(t)),
                current: named == pane(t), target: .go(ContentView.opening(worktree, terminal: t.id, in: fleet)))
        }
        func isLost(_ t: Terminal) -> Bool { LostPane.Kind(state: t.state) == .lost }
        let lost = own.filter(isLost).flatMap { t in
            [item(t)]
                + LostPane.actions(for: .lost).map { action in
                    JumpItem(
                        id: "lost|\(t.id)|\(action.title)", title: action.title, subtitle: t.label,
                        target: .lost(pane(t), action))
                }
        }
        return [
            JumpSection(title: "Terminals", items: own.filter { !isLost($0) }.map(item)),
            JumpSection(title: "Lost", items: lost),
        ]
    }
}

extension JumpMenus {
    /// Each crumb's menu, by its index: the first, a workspace's, its
    /// fellow workspaces; any other, a task or a History page, the
    /// workspace's level, on the place that crumb stands for. A loose
    /// worktree's lone crumb has none: its segment is the worktree's.
    static func crumbs(
        _ crumbs: [WorkspaceNavigation.Crumb], place: Selection, host: String, workspace: WorkspaceSummary?,
        board: TaskBoardModel, fleet: Fleet, groups: [WorkspaceNumbers.Group], showsHosts: Bool,
        waiting: (WorkspaceNumbers.Place) -> Int, tab: TaskTab?, chosen: (String) -> String?
    ) -> [JumpMenu] {
        guard let workspace else { return [] }
        let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: fleet)
        return crumbs.enumerated().map { index, crumb in
            if index == 0 {
                return workspaces(groups, current: (host, workspace.id), showsHosts: showsHosts, waiting: waiting)
            }
            let at = crumb.target ?? place
            return workspaceLevel(
                host: host, workspace: workspace.id, place: at, hasOrchestrator: !workspace.isImplicit,
                orchestrator: seat?.terminal.status, board: board,
                taskStatus: { row in
                    WorkspaceScreen.agent(of: row.id, host: host, in: fleet, chosen: chosen(row.id))?.terminal.status
                },
                taskWorktree: { WorkspaceWorktrees.worktree(of: $0, host: host, in: fleet)?.task },
                loose: WorkspaceWorktrees.loose(in: workspace, host: host, board: board, fleet: fleet).shown,
                worktreeStatus: \.attentionStatus, opening: { ContentView.opening($0, terminal: nil, in: fleet) },
                tab: crumb.target == nil ? tab : nil)
        }
    }
}

/// The crumbs' menus, for the bar to build when it needs them: how many
/// crumbs have one, and the builder, which reads the board and the fleet.
struct JumpMenuSource {
    var count = 0
    var build: () -> [JumpMenu] = { [] }

    static var none: JumpMenuSource { JumpMenuSource() }
}
