import AgentKit
import Foundation

// What the Plan view's sections read from the store (ov-503, phase 5), and the
// one write they make: taking a lane off a held train, through the CLI as every
// Mac write is (`farcooler plan train move`).

extension PlanStore {
    /// Whether the runner lists agents (`board_agents`): without it the Agents
    /// section isn't drawn, since "No agents running." would be false.
    var listsAgents: Bool { client.daemonBuild?.can(.boardAgentsList) == true }

    /// Whether the runner derives a train's phase and what holds it
    /// (`board_train_phase`), which is what makes a held train's button real.
    var movesLanes: Bool { client.daemonBuild?.can(.boardTrainPhase) == true }

    /// The Plan's sections for the plan as read, `needsYou` being the
    /// workspace's own count (the title bar's number).
    func sections(needsYou: Int, statuses: [String: TaskStatus], activity: [String: Int64]) -> PlanSections {
        PlanSections(plan: plan, needsYou: needsYou, statuses: statuses, activity: activity, agentsSupported: listsAgents)
    }

    /// Take `lane` off its train: to the next train when it's ready, else back
    /// to waiting. The runner decides which (`plan train move`), because the
    /// lane has to rebase; this asks, then reads the plan again. False when the
    /// runner refused, which the re-read then shows.
    @discardableResult
    func move(_ lane: PlanLane) async -> Bool {
        guard movesLanes else { return false }
        let moved = await client.moveLane(lane.name, repository: repositoryID, workspace: workspace.boardWorkspace)
        await reload()
        return moved
    }
}

extension DaemonClient {
    /// `plan train move <lane> --to next`, as the owner.
    func moveLane(_ lane: String, repository: String, workspace: String?) async -> Bool {
        let (data, _) = await runRaw(Self.moveArguments(lane, repository: repository, workspace: workspace), background: true)
        return data != nil
    }

    /// The command line for it, apart so a test can run the real CLI with the
    /// very arguments the app builds.
    static func moveArguments(_ lane: String, repository: String, workspace: String?) -> [String] {
        ["plan", "train", "move", lane, "--to", "next", "--repo", repository]
            + (workspace.map { ["--workspace", $0] } ?? []) + ["--actor", "user"]
    }
}
