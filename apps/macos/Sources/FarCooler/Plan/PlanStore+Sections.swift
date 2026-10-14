import AgentKit
import Foundation

// What the Plan view's sections read from the store (ov-503, phase 5).

extension PlanStore {
    /// Whether the runner lists agents (`board_agents`): without it the Agents
    /// section isn't drawn, since "No agents running." would be false.
    var listsAgents: Bool { client.daemonBuild?.can(.boardAgentsList) == true }

    /// The Plan's sections for the plan as read, `needsYou` being the
    /// workspace's own count (the title bar's number).
    func sections(needsYou: Int, statuses: [String: TaskStatus], activity: [String: Int64]) -> PlanSections {
        PlanSections(plan: plan, needsYou: needsYou, statuses: statuses, activity: activity, agentsSupported: listsAgents)
    }
}
