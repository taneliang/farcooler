import AgentKit
import Foundation

// Where a Needs You item takes you, and which one ⌃⌘N takes you to next.
//
// Every surface that opens an item asks this: the Needs You list's row, its
// Open button, and Next Needing Attention. So a click and the keyboard can't
// come to open the same item in two places. See spec §2.5 and §4.9.

enum NeedsYouNavigation {
    typealias Selection = ContentView.Selection

    /// The workspace `item` is counted under, as a selection names it: its own,
    /// or on a runner without `workstreams` its repository's implicit one. An
    /// item with neither is its repository's Main's, on a runner that lists
    /// one; else nil, and only its terminal says where it is.
    static func workspace(of item: NeedsYouItem, in fleet: Fleet) -> String? {
        if let id = item.workspaceID { return id }
        guard let repository = item.repositoryID else { return nil }
        guard let listed = fleet.runnerWorkspaces[item.runner] else { return repository }
        return listed.first { $0.isMain && $0.repository == repository }?.id
    }

    /// Where opening `item` lands (spec §2.5's last column):
    ///
    /// - one about a task, whatever its kind, is that task's column in its
    ///   workspace: its agent and changes, and a decision's card, which
    ///   starts expanded in Needs Decision;
    /// - an orchestrator's ask or block is its workspace's conversation;
    /// - any other agent's is where its pane lives (`WorkspaceSelection`).
    ///
    /// Nil only when none of that can be found any more.
    static func landing(for item: NeedsYouItem, in fleet: Fleet) -> Selection? {
        let host = item.runner
        if let task = item.task, let workspace = workspace(of: item, in: fleet) {
            return .workspace(host: host, workspace: workspace, focus: .task(task.id))
        }
        if let terminal = item.terminal {
            if terminal.isOrchestrator, let workspace = workspace(of: item, in: fleet) {
                return .workspace(host: host, workspace: workspace, focus: nil)
            }
            if let worktree = terminal.worktreeID
                ?? fleet.worktrees.first(where: { w in
                    (w.host ?? "") == host && w.terminals.contains { $0.id == terminal.id }
                })?.id,
                let landed = WorkspaceSelection.landing(
                    on: PaneRef(host: host, worktree: worktree, terminal: terminal.id), in: fleet)
            {
                return landed
            }
        }
        return workspace(of: item, in: fleet).map { .workspace(host: host, workspace: $0, focus: nil) }
    }

    /// The item ⌃⌘N opens after `current` (an item's `key`): the next in
    /// rank order, wrapping, or the first when `current` isn't listed any
    /// more. Items, not terminals: a finished agent is never one, and a
    /// decision, which no terminal holds, is.
    static func next(after current: String?, in items: [NeedsYouItem]) -> NeedsYouItem? {
        guard !items.isEmpty else { return nil }
        guard let current, let at = items.firstIndex(where: { $0.key == current }) else { return items.first }
        return items[(at + 1) % items.count]
    }
}
