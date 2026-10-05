import AgentKit
import AppKit
import SwiftUI

/// Workspaces and repositories: what the palette and the sidebar list, and starting a worktree, a main terminal or an orchestrator.
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    // MARK: - Workspaces and repositories

    /// Repositories New Workspace… can make one in: on runners with
    /// workspaces.
    var workspaceRepositories: [(host: String, repository: Repository)] {
        store.repositories.filter { store.fleet.runnerWorkspaces[$0.host] != nil }
    }

    /// Every workspace, for the palette, in the switcher's order
    /// (`WorkspaceDirectory`).
    var paletteWorkspaces: [PaletteWorkspace] {
        WorkspaceDirectory.groups(in: store.fleet).flatMap { group in
            group.workspaces.map { workspace in
                PaletteWorkspace(
                    host: group.host, id: workspace.id, name: workspace.isImplicit ? "Main" : workspace.name,
                    repository: group.project,
                    hasOrchestrator: WorkspaceScreen.orchestrator(of: workspace, host: group.host, in: store.fleet) != nil)
            }
        }
    }

    /// Every task on a board this window has read, for the palette.
    var paletteTasks: [PaletteTask] {
        boardStores.values.flatMap { board -> [PaletteTask] in
            let host = board.client.target
            let name = board.title
            return board.board.columns.flatMap(\.rows).map {
                PaletteTask(
                    host: host, workspace: board.workspace.id, workspaceName: name, id: $0.id, key: $0.key,
                    title: $0.title, status: $0.status)
            }
        }
    }

    /// Every theme and lane on the plans this window has read (ov-298).
    var palettePlans: [PalettePlanItem] {
        boardStores.values.flatMap { board -> [PalettePlanItem] in
            let host = board.client.target, id = board.workspace.id, name = board.title
            let plan = board.plan.plan
            let themes = plan.shownThemes.map {
                PalettePlanItem(
                    host: host, workspace: id, workspaceName: name, page: .theme($0.id), name: $0.name,
                    detail: PlanWords.progress($0.counts))
            }
            let lanes = plan.lanes.filter { $0.state != .dropped }.map {
                PalettePlanItem(
                    host: host, workspace: id, workspaceName: name, page: .lane($0.id), name: $0.name,
                    detail: PlanWords.state($0.state))
            }
            return themes + lanes
        }
    }

    /// A repository the switcher's Remove Repository… named, on its way to
    /// `RemoveRepositorySheet`.
    /// `.sheet(item:)` needs `Identifiable`; a bare tuple is not one.
    struct RepositoryToRemove: Identifiable {
        let host: String
        let repository: Repository
        var id: String { "\(host)\u{1}\(repository.id)" }
    }

    /// Whether to name runners at all.
    var showHosts: Bool { store.hosts.count > 1 }

    /// One runner's daemon, as something the runner item can offer to replace —
    /// or nil, which is the ordinary case and the quiet one.
    ///
    /// This is the only place that pairs a host with the client that can act
    /// on it, so the views downstream never learn whether a runner is updated
    /// over ssh or out of this app's own bundle. `DaemonClient.updateDaemon()`
    /// knows, and it is the only thing that needs to.
    ///
    /// Gated on `offersUpdate` rather than on "not current": a runner whose
    /// version could not be read is not a runner to offer an update for, and a
    /// runner nobody can reach is not one either. See `DaemonSkew`.
    func daemonUpdate(for host: String) -> DaemonUpdateTarget? {
        // Never for a runner ahead of this Mac (ov-143): updating it would
        // install this Mac's older build over it. The runner item says so
        // instead (`RunnerStatusItem`, `aheadHosts`).
        guard let client = store.clients[host], client.daemonSkew.offersUpdate else { return nil }
        return DaemonUpdateTarget(host: host, skew: client.daemonSkew) {
            await client.updateDaemon()
        }
    }

    /// The repository a project header stands for: by its uuid, which a
    /// worktree row carries as `repository_id`, and by its display name only
    /// from a CLI too old to send that — two repositories can share a name.
    func repository(host: String, id: String?, project: String) -> Repository? {
        let here = store.repositories.filter { $0.host == host }.map(\.repository)
        if let id { return here.first { $0.id == id } }
        return here.first { $0.displayName == project }
    }

    /// Start a worktree in a named project.
    ///
    /// The sheet already has a repository picker; this just answers it in
    /// advance, because someone clicking `+` on a project header has already
    /// said which one.
    func newWorktree(host: String, project: String) {
        newWorktreeIntent = NewWorktreeIntent(project: project, host: host)
    }

    /// A terminal in the repository's own checkout.
    ///
    /// The main checkout is always present in the fleet — the daemon adopts it
    /// the moment a repository is registered — so this finds it rather than
    /// asking the CLI to produce or locate it.
    private func newMainTerminal(host: String, repositoryID: String?, project: String) async {
        guard
            let worktree = Self.mainCheckout(
                host: host, repositoryID: repositoryID, project: project, in: store.fleet.worktrees)
        else { return }
        await act(.newTerminal, on: worktree) { client in
            await client.createTerminal(worktree: worktree.short, preset: "shell", title: "")
            await client.refresh()
        }
    }

    /// A repository's own checkout on `host`: by the repository's uuid, which
    /// a header carries as `repositoryID`, and by its display name only from
    /// a CLI too old to send one — two repositories on one runner can share a
    /// name, and the first of them is not the one whose header was clicked.
    static func mainCheckout(
        host: String, repositoryID: String?, project: String, in worktrees: [Worktree]
    ) -> Worktree? {
        worktrees.first {
            guard $0.isMainCheckout, ($0.host ?? "") == host else { return false }
            if let repositoryID { return $0.repositoryID == repositoryID }
            return $0.repository == project
        }
    }

    /// A plain, non-optional entry point into `newMainTerminal(host:repositoryID:project:)`,
    /// for the switcher's New Terminal in Checkout.
    func startMainTerminal(host: String, repositoryID: String?, project: String) {
        Task { await newMainTerminal(host: host, repositoryID: repositoryID, project: project) }
    }

    /// Move a worktree to another workspace (`farcooler worktree assign`):
    /// Move to Workspace ▸.
    func move(_ worktree: Worktree, to workspace: WorkspaceSummary) {
        Task {
            // Refused first, as every write here is; see `act`. Filed as
            // the move's own result, which a selection change leaves alone.
            let host = worktree.host ?? ""
            if let why = store.refusalSentence(for: host) {
                fileResult(.move, host: host, target: worktree.id, "\(ActionVerb.move.lead(Self.quoted(worktree))) \(why)")
                return
            }
            guard let client = store.client(for: worktree) else { return }
            // Nothing moves on screen until the runner says it has: a
            // refused move leaves the row where it was, with the sentence.
            let refused = await client.assignWorktree(worktree, to: workspace)
            fileResult(.move, host: host, target: worktree.id, refused)
        }
    }

    /// Start `workspace`'s orchestrator, or replace the one running. The row
    /// says "Starting Orchestrator…" until the runner answers; a refusal is
    /// the banner, in `orchestratorRefusal`'s words.
    func startOrchestrator(
        _ workspace: WorkspaceSummary, host: String, harness: OrchestratorHarness, replace: Bool
    ) {
        if let why = store.refusalSentence(for: host) {
            fileResult(.startOrchestrator, host: host, target: workspace.id,
                 "\(ActionVerb.startOrchestrator.lead(workspace.name)) \(why)")
            return
        }
        guard let client = store.clients[host] else { return }
        guard startingOrchestrators.begin(workspace, host: host) else { return }
        orchestratorStartedAt["\(host)|\(workspace.id)"] = Date()
        Task {
            let refused = await client.startOrchestrator(workspace, harness: harness, replace: replace)
            startingOrchestrators.end(workspace, host: host)
            fileResult(.startOrchestrator, host: host, target: workspace.id, refused)
        }
    }

    /// Every pane on one runner, for the board to find the ones working a
    /// task — or none while that runner is refused. See `BoardAgents.on`.
    func boardAgents(host: String, client: DaemonClient) -> BoardAgents {
        BoardAgents.on(
            store.fleet.worktrees.filter { ($0.host ?? "") == host },
            state: client.state, build: client.daemonBuild)
    }

    /// Why a selected board can't be drawn, said only as far as this app
    /// knows it: "gone" only from a runner that is answering and has listed
    /// its projects without it.
    func missingBoardSentence(host: String) -> String {
        guard let client = store.clients[host] else {
            return "The runner this board was on isn’t in Far Cooler anymore."
        }
        guard client.state == .connected, client.repositoriesListed else {
            return "Far Cooler is still loading this runner’s repositories."
        }
        return "This board isn’t on its runner anymore. Choose another workspace from the title bar."
    }

    /// Go to a pane a card offered, as the fleet has it now. See
    /// `BoardPane.landing`: its worktree when the pane has gone, and the
    /// board with a sentence when the worktree has too.
    func go(to pane: BoardPane) {
        guard let landed = BoardPane.landing(for: pane, in: store.fleet) else {
            errorBanner = "That agent has closed, and its worktree is gone."
            return
        }
        selection = landed
    }
}
