import AgentKit
import CFarCoolerClient
import Foundation

// What the Plan view's sections read from the store (ov-503, phase 5): the
// Rust core's view of the plan as read (ov-1016).

extension PlanStore {
    /// Whether the runner keeps the line (`board_line`): without it the Agents
    /// section isn't drawn, since "No agents running." would be false.
    var listsAgents: Bool { client.daemonBuild?.can(.boardLine) == true }

    /// The Plan's view for the plan as read, `needsYou` being the workspace's
    /// own count (the title bar's number); nil only when the bridge answers
    /// nothing, which a plan the runner sent never meets.
    func view(needsYou: Int, statuses: [String: TaskStatus], activity: [String: Int64]) -> WirePlanView? {
        PlanViewBridge.install()
        return WirePlanView.build(plan: plan, needsYou: needsYou, statuses: statuses, activity: activity, agentsSupported: listsAgents)
    }

    /// The plan column for the plan as read (ov-867), from the same view.
    func column(needsYou: Int = 0, statuses: [String: TaskStatus] = [:], titles: [String: String] = [:]) -> WireColumn? {
        PlanViewBridge.install()
        return WireColumn.of(plan, needsYou: needsYou, statuses: statuses, titles: titles)
    }
}

/// The Mac's call into the Rust core's `farcooler_client_plan_view`, which
/// AgentKit can't make itself (`PlanView.swift`). Naming the symbol here also
/// keeps it in the app's link.
enum PlanViewBridge {
    private static let installed: Void = PlanViewPresenter.install { PlanViewPresenter.call(farcooler_client_plan_view, $0) }

    /// Install it: at launch, and before any view is built, so a test that
    /// draws the Plan without launching the app has it too. Once.
    static func install() { _ = installed }
}
