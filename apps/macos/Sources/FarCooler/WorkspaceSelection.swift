import AgentKit
import Foundation

// What the window is showing, now that a workspace is a place (spec §4.2).
//
// The title bar's switcher and the sidebar list workspaces, and selecting one
// shows its navigator, with its orchestrator selected; a task, or one of its
// worktrees, selected there takes the main area (ov-85, ov-92). So a
// selection names a workspace and what's focused in it, nil for the
// orchestrator, and a worktree is a place only when no workspace owns it.
//
// Worked out here, as values, so the rules are the ones
// `WorkspaceSelectionTests` pins: where every old selection lands, and where
// "go to this terminal" lands from the palette, the attention cycle, a new
// worktree, or a notification.

extension ContentView {
    /// What the detail pane is showing.
    ///
    /// Carries the host alongside every id. A worktree's own id is a full
    /// per-daemon UUID and is not expected to collide across runners, but
    /// resolving a selection means finding both the thing AND the client that
    /// owns it, and `FleetStore.client(for:)` never routes by id alone.
    enum Selection: Hashable {
        /// Everything waiting on you, from every workspace (spec §4.6).
        case needsYou
        /// A workspace: its orchestrator's conversation beside its board,
        /// or `focus`, opened beside the board. On a runner without `workstreams`
        /// the workspace is a repository's implicit one, whose id is the
        /// repository's (`WorkspaceSummary.implicit`).
        case workspace(host: String, workspace: String, focus: Focus?)
        /// A worktree no workspace owns, or any worktree on a runner whose
        /// fleet doesn't say which repository it's in. `terminal` is the
        /// pane selected in it, or nil for the worktree as a whole.
        case looseWorktree(host: String, worktree: String, terminal: String?)

        /// Which runner it's on, or nil for Needs You, which is every
        /// runner's.
        var host: String? {
            switch self {
            case .needsYou: return nil
            case .workspace(let host, _, _), .looseWorktree(let host, _, _): return host
            }
        }

        /// The workspace it's in, or nil.
        var workspace: String? {
            if case .workspace(_, let id, _) = self { return id }
            return nil
        }

        /// The task or worktree open beside the board, or nil at the workspace's own
        /// level.
        var focus: Focus? {
            if case .workspace(_, _, let focus) = self { return focus }
            return nil
        }

        /// The same selection at the workspace's own level: where Back goes
        /// from a task, and the breadcrumb's first step.
        var closed: Selection {
            if case .workspace(let host, let id, _) = self { return .workspace(host: host, workspace: id, focus: nil) }
            return self
        }
    }

    /// What a workspace has open beside its board.
    enum Focus: Hashable {
        /// A task, by its id: its card, its agent and its changes (spec §4.4).
        case task(String)
        /// A worktree opened whole, from a task's Open Worktree or its row
        /// under the workspace, with the pane selected in it.
        case worktree(String, terminal: String?)
        /// A finished status's History page (ov-103): every task in Done or
        /// Canceled, grouped by when it landed, searchable.
        case history(TaskStatus)
    }
}

extension WorkspaceSelection {
    /// Whether two selections open the same thing: the same workspace with
    /// the same task or worktree open, or the same loose worktree, whichever
    /// of its panes is named.
    static func samePlace(_ a: ContentView.Selection?, _ b: ContentView.Selection?) -> Bool {
        a.map(place) == b.map(place)
    }

    /// `selection` with the pane it names left out: what's open, whichever
    /// of its panes is selected.
    static func place(_ selection: ContentView.Selection) -> ContentView.Selection {
        switch selection {
        case .workspace(let h, let w, .worktree(let wt, _)?):
            return .workspace(host: h, workspace: w, focus: .worktree(wt, terminal: nil))
        case .looseWorktree(let h, let wt, _):
            return .looseWorktree(host: h, worktree: wt, terminal: nil)
        default:
            return selection
        }
    }

    /// Whether going from `old` to `new` leaves `old`'s workspace: for
    /// another workspace, Needs You, a loose worktree beside another board,
    /// or nothing. Opening a task or a worktree beside its board, and
    /// closing it, stays in it, as does a loose worktree opened beside this
    /// workspace's own board (`beside`, the board `new` draws).
    static func leaves(_ old: ContentView.Selection?, for new: ContentView.Selection?, beside: String? = nil) -> Bool {
        guard case .workspace(let host, let id, _)? = old else { return false }
        if case .workspace(host, id, _)? = new { return false }
        if case .looseWorktree(host, _, _)? = new, beside == id { return false }
        return true
    }
}

/// One pane, on one runner, in the worktree its runner lists it in.
struct PaneRef: Hashable {
    var host: String
    var worktree: String
    var terminal: String
}

/// The selection as it was before workspaces were places: what
/// `fleet.lastTerminal` saved, and what a row, a palette pick or a jump used
/// to mean. Kept only to be mapped once, by `WorkspaceSelection.mapping`.
enum LegacySelection: Hashable {
    case worktree(host: String, id: String)
    case terminal(host: String, worktree: String, terminal: String)
    case board(host: String, workspace: String)

    /// `fleet.lastTerminal`'s string, `host/worktree/terminal`, with an empty
    /// host for this Mac. Nil for anything else.
    init?(lastTerminal saved: String) {
        // Empty pieces kept: this Mac's host is empty, so the string starts
        // with "/", and dropping that piece would shift every other one.
        let parts = saved.split(separator: "/", maxSplits: 2, omittingEmptySubsequences: false).map(String.init)
        guard parts.count == 3, !parts[1].isEmpty, !parts[2].isEmpty else { return nil }
        self = .terminal(host: parts[0], worktree: parts[1], terminal: parts[2])
    }
}

enum WorkspaceSelection {
    typealias Selection = ContentView.Selection

    /// Where an old selection lands now (spec §4.2's table):
    ///
    /// - a board is its workspace;
    /// - an orchestrator's terminal is its workspace, with nothing focused:
    ///   the conversation column is where it's drawn;
    /// - a terminal with a task, by `TaskLink`'s rule, is that task;
    /// - any other terminal, or a worktree, is that worktree under its owner,
    ///   or a loose worktree when no workspace owns it.
    ///
    /// Nil when its worktree isn't in the fleet any more.
    static func mapping(old: LegacySelection, in fleet: Fleet) -> Selection? {
        switch old {
        case .board(let host, let workspace):
            return .workspace(host: host, workspace: workspace, focus: nil)
        case .worktree(let host, let id):
            guard let worktree = worktree(host: host, id: id, in: fleet) else { return nil }
            return landing(in: worktree, terminal: nil, fleet: fleet)
        case .terminal(let host, let id, let terminal):
            guard let worktree = worktree(host: host, id: id, in: fleet) else { return nil }
            // A terminal that has closed since leaves its worktree, which is
            // still somewhere to go back to.
            let live = worktree.terminals.contains(where: { $0.id == terminal })
            return landing(in: worktree, terminal: live ? terminal : nil, fleet: fleet)
        }
    }

    /// Where going to `pane` lands: the same table as `mapping`, for a pane
    /// the fleet has now. What the palette, ⌃⌘N, a new worktree's first
    /// terminal and a board's Go to Agent all mean by "go to it".
    static func landing(on pane: PaneRef, in fleet: Fleet) -> Selection? {
        mapping(old: .terminal(host: pane.host, worktree: pane.worktree, terminal: pane.terminal), in: fleet)
    }

    /// Where a worktree, or a pane in it, lands.
    static func landing(in worktree: Worktree, terminal id: String?, fleet: Fleet) -> Selection {
        let host = worktree.host ?? ""
        let terminal = id.flatMap { id in worktree.terminals.first { $0.id == id } }
        if let terminal, terminal.isOrchestrator,
            let workspace = terminal.workspace ?? owner(of: worktree, in: fleet)
        {
            return .workspace(host: host, workspace: workspace, focus: nil)
        }
        let owner = owner(of: worktree, in: fleet)
        if let terminal, let task = TaskLink.task(of: terminal, in: worktree),
            let workspace = listed(terminal.workspace, host: host, in: fleet) ?? owner
        {
            return .workspace(host: host, workspace: workspace, focus: .task(task))
        }
        guard let owner else { return .looseWorktree(host: host, worktree: worktree.id, terminal: id) }
        return .workspace(host: host, workspace: owner, focus: .worktree(worktree.id, terminal: id))
    }

    /// The workspace `worktree` is drawn under, or nil when it's loose.
    ///
    /// On a runner with `workstreams`, its owner when that runner lists it;
    /// an unclaimed worktree, or one whose owner the runner no longer lists,
    /// is loose. On a runner without them, its repository's implicit
    /// workspace, whose id is the repository's; loose only when the CLI is
    /// too old to say which repository that is.
    static func owner(of worktree: Worktree, in fleet: Fleet) -> String? {
        let host = worktree.host ?? ""
        guard fleet.runnerWorkspaces[host] != nil else { return worktree.repositoryID }
        return listed(worktree.workspace, host: host, in: fleet)
    }

    /// `id`, when `host`'s runner lists a workspace by it.
    private static func listed(_ id: String?, host: String, in fleet: Fleet) -> String? {
        guard let id, let listed = fleet.runnerWorkspaces[host] else { return nil }
        return listed.contains(where: { $0.id == id }) ? id : nil
    }

    static func worktree(host: String, id: String, in fleet: Fleet) -> Worktree? {
        fleet.worktrees.first { ($0.host ?? "") == host && $0.id == id }
    }
}

extension ContentView {
    /// What of `worktree` a selection shows, for its sidebar row: the
    /// worktree whole (`.some(nil)`), one of its terminals, or nothing.
    nonisolated static func selected(in worktree: Worktree, by selection: Selection?) -> String?? {
        let host = worktree.host ?? ""
        switch selection {
        case .looseWorktree(host, worktree.id, let terminal),
            .workspace(host, _, .worktree(worktree.id, let terminal)):
            return .some(terminal)
        default:
            return nil
        }
    }

    /// Where choosing `worktree`, or a terminal in it, from its row goes: the
    /// workspace that owns it, with the worktree open, or the worktree alone when none
    /// does. Never its task: the row was the worktree's.
    nonisolated static func opening(_ worktree: Worktree, terminal: String?, in fleet: Fleet) -> Selection {
        let host = worktree.host ?? ""
        guard let owner = WorkspaceSelection.owner(of: worktree, in: fleet) else {
            return .looseWorktree(host: host, worktree: worktree.id, terminal: terminal)
        }
        return .workspace(host: host, workspace: owner, focus: .worktree(worktree.id, terminal: terminal))
    }

    /// The worktree a selection opens whole, with no pane named in it.
    nonisolated static func openedWhole(_ selection: Selection?) -> (host: String, worktree: String)? {
        switch selection {
        case .looseWorktree(let host, let id, nil), .workspace(let host, _, .worktree(let id, nil)):
            return (host, id)
        default:
            return nil
        }
    }
}

/// A workspace's needs-you count, for its sidebar row.
enum WorkspaceCounts {
    /// Its items: those counted under it on its runner. A repository's
    /// implicit workspace, on a runner without `workstreams`, counts its
    /// repository's items with no workspace.
    static func count(for workspace: WorkspaceSummary, host: String, in items: [NeedsYouItem]) -> Int {
        items.filter { counted($0, under: workspace, host: host) }.count
    }

    /// The board's "N tasks are waiting on you": its items that are a
    /// decision, alone or beside an ask. Not the Needs Decision column's
    /// rows, which an answer leaves as they were (spec §2.2).
    static func decisions(for workspace: WorkspaceSummary, host: String, in items: [NeedsYouItem]) -> Int {
        items.filter { item in
            counted(item, under: workspace, host: host) && (item.kind == .decision || item.also.contains(.decision))
        }.count
    }

    /// What the board's pill says: the decision items once this runner's
    /// list is read, and the Needs Decision column's count until then, and
    /// always on a runner that serves no list. An unread list is not a list
    /// with nothing in it.
    static func waiting(columnCount: Int, decisions: Int, listRead: Bool, listServed: Bool) -> Int {
        listRead && listServed ? decisions : columnCount
    }

    private static func counted(_ item: NeedsYouItem, under workspace: WorkspaceSummary, host: String) -> Bool {
        guard item.runner == host else { return false }
        if workspace.isImplicit { return item.workspaceID == nil && item.repositoryID == workspace.id }
        return item.workspaceID == workspace.id
    }
}
