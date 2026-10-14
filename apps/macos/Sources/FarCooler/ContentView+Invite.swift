import AgentKit
import SwiftUI

/// The window's way to an orchestrator for the empty states (ov-519).
extension ContentView {
    /// Where an orchestrator would run, as a sentence names it.
    func runnerPlace(_ host: String) -> RunnerPlace {
        host.isEmpty ? .thisMac : .runner(ReadOnlyFolders.runnerName(host))
    }

    /// The invitation for `workspace`: start its orchestrator, or show it.
    /// Nil on a runner without workspaces, where none can run.
    func orchestratorInvite(host: String, workspace: WorkspaceSummary?) -> OrchestratorInvite? {
        guard let workspace, !workspace.isImplicit, let client = store.clients[host] else { return nil }
        let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet)
        return OrchestratorInvite(
            seated: seat != nil || startingOrchestrators.isStarting(workspace, host: host),
            canAct: store.refusal(for: host) == nil,
            availability: client.availability,
            place: runnerPlace(host),
            onStart: { startOrchestrator(workspace, host: host, harness: $0, replace: false) },
            onShow: { selectOrchestrator(keyboard: .conversation) },
            onCheckAgain: { Task { await client.recheckInstalled() } })
    }

    /// The invitation Needs You offers, which is no workspace's: where no
    /// orchestrator runs anywhere, Start Orchestrator starts one in the first
    /// workspace, and checks the agent is installed on that workspace's runner.
    var needsYouInvite: OrchestratorInvite? {
        let workspaces: [(host: String, summary: WorkspaceSummary)] = store.fleet.runnerWorkspaces
            .sorted { $0.key < $1.key }
            .flatMap { host, listed in listed.filter { !$0.isImplicit }.map { (host, $0) } }
        guard let first = workspaces.first else { return nil }
        let seated = workspaces.contains {
            WorkspaceScreen.orchestrator(of: $0.summary, host: $0.host, in: store.fleet) != nil
        }
        let client = store.clients[first.host]
        return OrchestratorInvite(
            seated: seated, canAct: store.refusal(for: first.host) == nil,
            availability: client?.availability ?? HarnessAvailability(agentsFound: nil),
            place: runnerPlace(first.host),
            onStart: { startOrchestrator(first.summary, host: first.host, harness: $0, replace: false) },
            onCheckAgain: { Task { await client?.recheckInstalled() } })
    }
}
