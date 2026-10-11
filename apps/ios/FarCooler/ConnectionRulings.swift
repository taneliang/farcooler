import Foundation

// The owner's marks on a ruling, from the iPhone (ov-333). Keep is the
// owner's own mark: `ruling.keep` through the client core, which sends
// `ruling.set` to confirmed as the user, and never reaches the orchestrator.
// Reverse and Discuss do, through `RulingActions` (AgentKit), which holds the
// rules and the words.

extension Connection {
    /// Keep one ruling: shown kept at once, then asked of the runner and the
    /// plan read again, which puts it back if the runner refused.
    func keepRuling(_ ruling: PlanRuling, in summary: WorkspaceSummary) async {
        plans.showKept([ruling.id], in: summary.id)
        _ = try? await rpc("ruling.keep", ["ruling": ruling.id])
        await readPlan(summary)
    }

    /// Keep every open ruling on the board.
    func keepAllRulings(in summary: WorkspaceSummary) async {
        guard case .loaded(let plan)? = plans.state(summary.id), let board = summary.boardWorkspace else { return }
        plans.showKept(Set(plan.openRulings.map(\.id)), in: summary.id)
        _ = try? await rpc("ruling.keep_all", ["workspace": board])
        await readPlan(summary)
    }
}
