import AgentKit
import Foundation

// Which panes the detail puts on screen for a selection, column by column.
//
// A workspace shows up to two tmux layouts at once: its orchestrator's, in the
// conversation column, and a task's agent or an opened worktree in the third.
// Everything that asks "what is on screen" asks this, so the layout commands,
// ⌘] and ⌘[, "seen", and the drawing itself can't come to disagree about it:
// the one mistake worse than not marking a pane seen is marking one that isn't
// showing.

/// One tmux layout the detail draws, and where.
struct ShownLayout: Equatable {
    enum Column: Hashable {
        /// The workspace's orchestrator (spec §4.10).
        case conversation
        /// A task's agent (spec §4.4).
        case task
        /// A worktree opened whole: from the Worktrees disclosure, a task's
        /// Open Worktree, or a loose worktree.
        case worktree
    }

    var column: Column
    var worktree: Worktree
    var group: PaneGroup
    /// The layouts its bar offers: the worktree's own layouts for a worktree
    /// opened whole, and the one layout otherwise. A task's agent is shown as
    /// the one layout holding it, and an orchestrator's window is its own.
    var groups: [PaneGroup]

    var host: String { worktree.host ?? "" }

    func contains(_ pane: PaneRef) -> Bool {
        pane.host == host && pane.worktree == worktree.id && group.terminals.contains(pane.terminal)
    }
}

enum WorkspaceScreen {
    /// A workspace, as its runner lists it now: a listed one, or on a runner
    /// without workspaces the repository whose id it is. Nil once it's gone.
    static func workspace(_ id: String, host: String, in fleet: Fleet, repositories: [String]) -> WorkspaceSummary? {
        if let listed = fleet.runnerWorkspaces[host] { return listed.first { $0.id == id } }
        guard repositories.contains(id) else { return nil }
        return .implicit(repository: id)
    }

    /// Who `workspace`'s conversation column shows: the runner's live seat,
    /// `WorkspaceSummary.orchestrator`, first. Only when the runner names none
    /// is a terminal taken by its role, and then only a live one: a
    /// workspace can hold a stopped orchestrator beside the one that replaced
    /// it, and the column must never show the stopped one over it.
    static func orchestrator(of workspace: WorkspaceSummary, host: String, in fleet: Fleet) -> BoardPane? {
        guard !workspace.isImplicit else { return nil }
        if let seat = workspace.orchestrator, let pane = pane(seat, host: host, in: fleet) { return pane }
        for worktree in fleet.worktrees where (worktree.host ?? "") == host {
            if let live = worktree.terminals.first(where: {
                $0.isOrchestrator && $0.workspace == workspace.id
                    && [.running, .starting].contains(StateKind.parse($0.state))
            }) {
                return BoardPane(terminal: live, worktree: worktree)
            }
        }
        return nil
    }

    /// The panes working task `id` on `host`, in the runner's order: by
    /// `TaskAgentLink.isWorking`, the board's own rule, so the column and the
    /// card's Agent pill agree about the same task.
    static func agents(of id: String, host: String, in fleet: Fleet) -> [BoardPane] {
        fleet.worktrees.filter { ($0.host ?? "") == host }.flatMap { worktree in
            worktree.terminals
                .filter { TaskAgentLink.isWorking($0, on: id) }
                .map { BoardPane(terminal: $0, worktree: worktree) }
        }
    }

    /// The agent a task's column shows: the one chosen from its picker when
    /// that one is still working the task, else the first.
    static func agent(of id: String, host: String, in fleet: Fleet, chosen: String?) -> BoardPane? {
        let all = agents(of: id, host: host, in: fleet)
        return all.first { $0.terminal.id == chosen } ?? all.first
    }

    /// A terminal on `host`, with the worktree it's in.
    static func pane(_ terminal: String, host: String, in fleet: Fleet) -> BoardPane? {
        for worktree in fleet.worktrees where (worktree.host ?? "") == host {
            if let found = worktree.terminals.first(where: { $0.id == terminal }) {
                return BoardPane(terminal: found, worktree: worktree)
            }
        }
        return nil
    }

    /// Every layout the detail draws for `selection`, conversation first.
    ///
    /// `layouts` answers a worktree's layouts on its runner, nil before the
    /// first read; `chosen` is the agent picked in a task column's picker.
    static func shown(
        _ selection: ContentView.Selection?, in fleet: Fleet,
        layouts: (_ host: String, _ worktree: String) -> [PaneGroup]?,
        repositories: (_ host: String) -> [String] = { _ in [] },
        chosen: (_ task: String) -> String? = { _ in nil }
    ) -> [ShownLayout] {
        func holding(_ pane: BoardPane) -> PaneGroup? {
            (layouts(pane.worktree.host ?? "", pane.worktree.id) ?? []).first {
                $0.terminals.contains(pane.terminal.id)
            }
        }
        /// A worktree opened whole: the layout holding `terminal`, else its
        /// own active layout, with its own layouts in the bar.
        func opened(_ worktree: Worktree, terminal: String?) -> ShownLayout? {
            let all = layouts(worktree.host ?? "", worktree.id) ?? []
            let own = ContentView.ownLayouts(all, of: worktree)
            if let terminal, let group = all.first(where: { $0.terminals.contains(terminal) }) {
                return ShownLayout(
                    column: .worktree, worktree: worktree, group: group,
                    groups: own.contains { $0.id == group.id } ? own : [group])
            }
            guard let group = ContentView.shownLayout(all, of: worktree), !group.terminals.isEmpty else {
                return nil
            }
            return ShownLayout(column: .worktree, worktree: worktree, group: group, groups: own)
        }

        switch selection {
        case nil, .needsYou:
            return []
        case .looseWorktree(let host, let id, let terminal):
            guard let worktree = WorkspaceSelection.worktree(host: host, id: id, in: fleet) else { return [] }
            return opened(worktree, terminal: terminal).map { [$0] } ?? []
        case .workspace(let host, let id, let focus):
            var out: [ShownLayout] = []
            if let workspace = workspace(id, host: host, in: fleet, repositories: repositories(host)),
                let seat = orchestrator(of: workspace, host: host, in: fleet),
                let group = holding(seat)
            {
                out.append(ShownLayout(column: .conversation, worktree: seat.worktree, group: group, groups: [group]))
            }
            switch focus {
            case nil:
                break
            case .task(let task):
                if let agent = agent(of: task, host: host, in: fleet, chosen: chosen(task)),
                    let group = holding(agent)
                {
                    out.append(ShownLayout(column: .task, worktree: agent.worktree, group: group, groups: [group]))
                }
            case .worktree(let worktreeID, let terminal):
                if let worktree = WorkspaceSelection.worktree(host: host, id: worktreeID, in: fleet),
                    let shown = opened(worktree, terminal: terminal)
                {
                    out.append(shown)
                }
            }
            return out
        }
    }

    /// The pane the keyboard acts on: `key`, the pane last clicked or
    /// focused, while it's on screen; else the third column's focused pane,
    /// else the conversation's.
    ///
    /// Picked from what's shown so ⌃B, ⌘W and ⌘] act on a pane you can see.
    /// The third column leads when there is one, because opening a task or a
    /// worktree is going there.
    static func keyPane(_ key: PaneRef?, in shown: [ShownLayout], selection: ContentView.Selection?) -> PaneRef? {
        if let key, shown.contains(where: { $0.contains(key) }) { return key }
        guard let front = shown.last else { return nil }
        let named: String? = {
            switch selection {
            case .looseWorktree(_, _, let terminal): return terminal
            case .workspace(_, _, .worktree(_, let terminal)): return terminal
            default: return nil
            }
        }()
        let terminal =
            named.flatMap { front.group.terminals.contains($0) ? $0 : nil }
            ?? front.group.focused ?? front.group.terminals.first
        return terminal.map { PaneRef(host: front.host, worktree: front.worktree.id, terminal: $0) }
    }
}

extension ContentView {
    /// What the window's title says while `shown` is the layout in front.
    ///
    /// An orchestrator's pane is in the main checkout only because that's
    /// where the runner opens it, so it's titled with its workspace, as the
    /// board is. Anything else is titled with its worktree.
    static func frame(of shown: ShownLayout, in fleet: Fleet) -> (title: String, subtitle: String) {
        let worktree = shown.worktree
        if shown.column == .conversation,
            let terminal = shown.group.terminals.lazy.compactMap({ id in worktree.terminals.first { $0.id == id } })
                .first(where: \.isOrchestrator),
            let id = terminal.workspace,
            let workspace = fleet.runnerWorkspaces[shown.host]?.first(where: { $0.id == id })
        {
            let subtitle = [worktree.repository, "Orchestrator", shown.host.isEmpty ? nil : shown.host]
                .compactMap { $0 }
                .joined(separator: " · ")
            return (workspace.name, subtitle)
        }
        return (worktree.windowTitle, worktree.windowSubtitle)
    }
}
