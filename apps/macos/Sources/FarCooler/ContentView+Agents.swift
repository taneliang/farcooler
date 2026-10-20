import AgentKit
import SwiftUI

/// The cards over a workspace's conversation (ov-641) and where an agent in
/// the Agents panel goes.
extension ContentView {
    /// The agents at work for `workspace` on `host`: the fleet's count, with
    /// detail from each conversation this window follows live.
    func runningAgents(host: String, workspace: String) -> RunningAgents {
        MacAgents.running(workspace: workspace, host: host, fleet: store.fleet) { MacAgents.followed($0, in: nativeAgents) }
    }

    /// The row of cards over `scene`'s conversation: Now/Next (`strip`, the
    /// folded window's) and Agents beside it, or Agents alone when the canvas
    /// is beside the chat.
    func workspaceCards(_ scene: WorkspaceScene, strip: AnyView?, canvas: Bool) -> AnyView {
        guard let summary = scene.summary, scene.hasConversation else { return AnyView(EmptyView()) }
        let host = scene.host
        let agents = runningAgents(host: host, workspace: summary.id)
        return AnyView(
            WorkspaceCardsRow(
                agents: agents, showsPlan: !canvas && strip != nil,
                onChoose: { openAgent($0, host: host, workspace: summary) }
            ) { strip ?? AnyView(EmptyView()) })
    }

    /// Choosing an agent: a pane opens in its worktree, as the board's agent
    /// pills do; a subagent opens its own conversation in its parent's place
    /// where the runner serves it, else its parent's pane. The orchestrator's
    /// own is the workspace's conversation column.
    func openAgent(_ agent: RunningAgent, host: String, workspace: WorkspaceSummary) {
        let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet)?.terminal.id
        let next = Self.destination(of: agent, seat: seat, host: host, workspace: workspace.id)
        if next != selection {
            trail = nil
            focusColumn = false
            selection = next
        }
        if agent.opensOwnConversation, let row = agent.row, let agentId = agent.agentId {
            _ = nativeAgents.model(ifMade: agent.terminal)?.openAgent(row: row, agentId: agentId)
        }
    }

    /// Where `agent` is: the workspace's own conversation when it's the
    /// orchestrator's (`seat`) or one of its subagents, else its pane in its
    /// worktree.
    nonisolated static func destination(of agent: RunningAgent, seat: String?, host: String, workspace: String) -> Selection {
        agent.terminal == seat
            ? .workspace(host: host, workspace: workspace, focus: nil)
            : .workspace(host: host, workspace: workspace, focus: .worktree(agent.worktree, terminal: agent.terminal))
    }
}
