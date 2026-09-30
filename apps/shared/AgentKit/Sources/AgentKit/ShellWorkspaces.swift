import Foundation

// What a workspace's orchestrator is called on the shell's bar.
//
// A runner with `workstreams` has an orchestrator per workspace, and its pane
// runs in a worktree — Main's checkout — that another workspace may own. A tab
// called after its program would read as one more terminal of that worktree's
// in the bar, so it is called after the workspace it belongs to. The grouping
// itself is `WorkspaceGrouping`'s, shared with the Mac and mirrored by
// Android's `model/FleetLayout.kt`; what this file adds is what the phone
// draws from it, which is a rule a view body would otherwise decide and no
// suite would read. See `ShellWorkspacesTests`.

extension ShellTab {
    /// An orchestrator's tab title: its workspace's name, then "Orchestrator".
    static func orchestratorTitle(workspace: String) -> String { "\(workspace) Orchestrator" }
}

extension Fleet {
    /// What each orchestrator's tab is called, by its terminal id: "Billing
    /// Orchestrator". Nil for a runner without `workstreams`, which has no
    /// workspaces to name one after.
    ///
    /// An orchestrator is its workspace's, found by its pane's role, and only
    /// when its terminal is in this fleet: a `WorkspaceSummary.orchestrator`
    /// naming a pane the fleet has not listed would be a title for a tab that
    /// does not exist.
    func orchestratorTitles() -> [String: String]? {
        guard workspaces != nil else { return nil }
        let terminals = Set(worktrees.flatMap { $0.terminals.map(\.id) })
        var titles: [String: String] = [:]
        for repository in repositoryGroups() {
            for group in repository.workspaces {
                guard let id = group.orchestrator, terminals.contains(id) else { continue }
                titles[id] = ShellTab.orchestratorTitle(workspace: group.workspace.name)
            }
        }
        return titles
    }
}
