import AgentKit
import Foundation

// Rules about a fleet's workspaces that the window, the title bar's switcher
// and the palette share (ov-178, where the old Fleet sidebar that drew them
// went): which workspaces there are, in the order ⌘1–9 numbers them; where
// a worktree can move; a checkout's own layouts; whose board a selection is
// beside, and which workspace a new worktree is claimed for.
//
// Which workspace owns which worktree, and in what order workspaces come, is
// AgentKit's rule (`WorkspaceGrouping`), shared with the phones.

/// Every workspace on every runner, by repository, in the order the
/// switcher lists them and ⌘1–9 numbers them: repositories in the runner's
/// worktree order, and each one's workspaces Main first, then by ordinal
/// (`WorkspaceGrouping`). The order the old sidebar drew, kept so nobody's
/// numbers move.
enum WorkspaceDirectory {
    /// One repository on one runner, and its workspaces.
    struct Group: Equatable {
        var host: String
        /// The repository's display name.
        var project: String
        /// Its uuid, or nil from a CLI too old to send one.
        var repositoryID: String?
        var workspaces: [WorkspaceSummary] = []
    }

    /// Every repository with a workspace, in order. A worktree with no
    /// repository has none, and a repository whose runner lists workspaces
    /// but none of its own has none either.
    static func groups(in fleet: Fleet) -> [Group] {
        var order: [String] = []
        var groups: [String: Group] = [:]
        for worktree in fleet.worktrees {
            guard let project = worktree.repository, !project.isEmpty else { continue }
            let host = worktree.host ?? ""
            // By uuid where the CLI sends one: two repositories that share
            // a name are two groups.
            let key = "\(host)\u{1}\(worktree.repositoryID ?? project)"
            if groups[key] == nil {
                order.append(key)
                groups[key] = Group(host: host, project: project, repositoryID: worktree.repositoryID)
            }
        }
        return order.compactMap { key in
            guard var group = groups[key] else { return nil }
            group.workspaces = workspaces(host: group.host, repositoryID: group.repositoryID, in: fleet)
            return group.workspaces.isEmpty ? nil : group
        }
    }

    /// A repository's workspaces, Main first: a runner without workspaces
    /// (`fleet.runnerWorkspaces` has no key for it) has one implicit one.
    static func workspaces(host: String, repositoryID: String?, in fleet: Fleet) -> [WorkspaceSummary] {
        guard let listed = fleet.runnerWorkspaces[host] else {
            return repositoryID.map { [.implicit(repository: $0)] } ?? []
        }
        let mine = listed.filter { $0.repository != nil && $0.repository == repositoryID }
        guard !mine.isEmpty else { return [] }
        return WorkspaceGrouping.group(
            repository: repositoryID ?? "", workspaces: mine, worktrees: [], orchestrators: [:]
        ).workspaces.map(\.workspace)
    }
}

extension ContentView {
    /// What Move to Workspace ▸ offers for `worktree` (`farcooler worktree
    /// assign`): the other workspaces of its repository, on its runner.
    /// None for the main checkout, which is the repository's own directory
    /// where every workspace's orchestrator runs, not one workspace's
    /// worktree; and none on a runner without `workstreams` (`assigns`),
    /// which has no command for it. Nothing un-assigns a worktree, so
    /// Unclaimed is never a target.
    static func moveTargets(for worktree: Worktree, in fleet: Fleet, assigns: Bool) -> [WorkspaceSummary] {
        guard assigns, !worktree.isMainCheckout, let repository = worktree.repositoryID else { return [] }
        return (fleet.runnerWorkspaces[worktree.host ?? ""] ?? []).filter {
            $0.repository == repository && $0.id != worktree.workspace
        }
    }

    /// Whether a worktree's runner can take `worktree assign`, as `store`
    /// knows it at the moment of asking.
    static func assigns(_ store: FleetStore) -> (Worktree) -> Bool {
        { store.client(for: $0)?.daemonBuild?.can(.workstreams) ?? false }
    }

    /// The orchestrators the runner lists in `worktree`, by their role:
    /// what `ownLayouts` leaves out of the checkout they run in.
    ///
    /// By role rather than by the workspaces' seats. There are none when the
    /// fleet read carries no workspaces for the runner (an older CLI, or
    /// before the first read), while the orchestrators' windows are still in
    /// the checkout's session. `worktree` is the runner's, not a
    /// row's: a row's has the orchestrators drawn elsewhere taken out.
    static func orchestrators(in worktree: Worktree) -> Set<String> {
        Set(worktree.terminals.filter(\.isOrchestrator).map(\.id))
    }

    /// `worktree`'s layouts without its orchestrators' windows.
    ///
    /// The runner opens every workspace's orchestrator as a tmux window in
    /// the main checkout's session, so the checkout's layouts include
    /// Billing's orchestrator. It is reached through its own workspace; offered in
    /// the checkout's bar, or picked as the checkout's layout because tmux
    /// calls it the active window since it was last focused, it would put
    /// Billing's orchestrator under Main's checkout.
    ///
    /// A stopped orchestrator the runner no longer seats is drawn among the
    /// checkout's terminals, and its window is left out all the same:
    /// selecting it shows that window alone. See `WorkspaceScreen.shown`.
    static func ownLayouts(_ groups: [PaneGroup], of worktree: Worktree) -> [PaneGroup] {
        let orchestrators = orchestrators(in: worktree)
        guard !orchestrators.isEmpty else { return groups }
        return groups.filter { !$0.terminals.contains(where: orchestrators.contains) }
    }

    /// The layout selecting a worktree's own row shows: the active one of its
    /// own layouts, else the first of them. Nil when it has none.
    static func shownLayout(_ groups: [PaneGroup]?, of worktree: Worktree) -> PaneGroup? {
        let own = ownLayouts(groups ?? [], of: worktree)
        return own.first { $0.isActive } ?? own.first
    }

    /// The pane ⌃B and a digit focuses: that pane of the layout on screen,
    /// counted in tmux's pane order, or nil past its last.
    ///
    /// Picked here rather than by the runner's `layout focus --pane`, which
    /// counts in the window tmux calls active — an orchestrator's, if it was
    /// the last one focused, while the checkout's own layout is on screen.
    static func pane(numbered number: Int, in group: PaneGroup?) -> PaneRect? {
        guard let group, number >= 1, number <= group.panes.count else { return nil }
        return group.panes[number - 1]
    }

    /// The layout ⌃B n or ⌃B p lands on from `current`: the next or previous
    /// of `groups`, wrapping, or nil when there is nowhere else to go.
    ///
    /// Stepped here, through the layouts the bar offers, rather than by the
    /// runner's `layout select --next`, which walks every window in the
    /// session — from the main checkout, into the orchestrators'.
    static func layout(stepping step: Int, from current: String?, in groups: [PaneGroup]) -> PaneGroup? {
        guard groups.count > 1, let current, let index = groups.firstIndex(where: { $0.id == current })
        else { return nil }
        return groups[((index + step) % groups.count + groups.count) % groups.count]
    }

    /// The workspace `selection` is in, on its runner, as far as the fleet
    /// says: a board's own, the orchestrator's for its row, and otherwise
    /// the worktree's owner, or nil for an unclaimed worktree.
    private static func workspaceID(
        of selection: Selection, in fleet: Fleet
    ) -> (host: String, worktree: Worktree?, id: String?)? {
        switch selection {
        case .needsYou:
            return nil
        case .workspace(let host, let id, _):
            return (host, nil, id)
        case .looseWorktree(let host, let id, _):
            guard
                let worktree = fleet.worktrees.first(where: { ($0.host ?? "") == host && $0.id == id })
            else { return nil }
            return (host, worktree, worktree.workspace)
        }
    }

    /// Whose board ⇧⌘B opens from `selection`: the workspace it is in, or
    /// Main's when that is an unclaimed worktree — the board every
    /// repository has. On a runner without workspaces, the repository's one
    /// board.
    ///
    /// Nil when the selection isn't in the fleet, and for a worktree on a
    /// runner without workspaces whose CLI sent no repository id — the
    /// caller then looks its repository up by name.
    static func boardWorkspace(for selection: Selection, in fleet: Fleet) -> WorkspaceSummary? {
        guard let found = workspaceID(of: selection, in: fleet) else { return nil }
        let (host, worktree, id) = found
        guard let listed = fleet.runnerWorkspaces[host] else {
            // No workspaces: a board selection's id is its repository's.
            let repository = worktree.map(\.repositoryID) ?? id
            return repository.map(WorkspaceSummary.implicit(repository:))
        }
        if let id, let found = listed.first(where: { $0.id == id }) { return found }
        guard let worktree else { return nil }
        return listed.first { $0.isMain && $0.repository != nil && $0.repository == worktree.repositoryID }
    }

    /// The workspace a new worktree in `repository` on `host` is claimed for:
    /// the one `selection` is in, when it is in that repository on that
    /// runner, and otherwise the repository's Main — an unclaimed worktree,
    /// another repository or runner, or nothing selected. Never left
    /// unclaimed where the runner has workspaces: a pane opened in an
    /// unclaimed worktree has no workspace, so nothing done there could ever
    /// claim it. Nil only on a runner without workspaces, which has nothing
    /// to claim for.
    static func claim(
        newWorktreeIn repository: String, on host: String, from selection: Selection?,
        in fleet: Fleet
    ) -> String? {
        guard let listed = fleet.runnerWorkspaces[host] else { return nil }
        if let selection, let found = workspaceID(of: selection, in: fleet), found.host == host,
            let id = found.id,
            let workspace = listed.first(where: { $0.id == id }), workspace.repository == repository
        {
            return workspace.id
        }
        return listed.first { $0.isMain && $0.repository == repository }?.id
    }
}
