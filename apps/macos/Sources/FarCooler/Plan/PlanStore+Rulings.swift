import AgentKit
import Foundation

// The owner's Keep and Keep All (ov-333), through the CLI as every Mac write
// is: `farcooler plan ruling keep`. Keep is the owner's own mark, so it never
// reaches the orchestrator and never changes the plan; Reverse and Discuss do
// reach it, from the window (`ContentView.planRulingActions`).

extension PlanStore {
    /// Whether this runner takes the owner's marks on a ruling.
    var canMarkRulings: Bool { client.daemonBuild?.can(.boardRulingActions) == true }

    /// Keep `ruling`: shown kept at once, then asked of the runner and read
    /// again. False when the runner refused, which the re-read then shows.
    @discardableResult
    func keep(_ ruling: PlanRuling) async -> Bool {
        guard canMarkRulings, ruling.isStanding else { return false }
        showKept([ruling.id])
        let kept = await client.keepRuling(ruling.short, repository: repositoryID, workspace: workspace.boardWorkspace)
        await reload()
        return kept
    }

    /// Keep every open ruling on this board.
    @discardableResult
    func keepAll() async -> Bool {
        guard canMarkRulings, !plan.openRulings.isEmpty else { return false }
        showKept(Set(plan.openRulings.map(\.id)))
        let kept = await client.keepRuling(nil, repository: repositoryID, workspace: workspace.boardWorkspace)
        await reload()
        return kept
    }
}

extension DaemonClient {
    /// `plan ruling keep R-12`, or `--all` for none named: as the owner,
    /// whatever `FARCOOLER_ACTOR` this app was launched under.
    func keepRuling(_ short: String?, repository: String, workspace: String?) async -> Bool {
        let (data, _) = await runRaw(
            Self.keepArguments(short, repository: repository, workspace: workspace), background: true)
        return data != nil
    }

    /// The command line for it, apart so a test can run the real CLI with the
    /// very arguments the app builds.
    static func keepArguments(_ short: String?, repository: String, workspace: String?) -> [String] {
        ["plan", "ruling", "keep"] + (short.map { [$0] } ?? ["--all"]) + ["--repo", repository]
            + (workspace.map { ["--workspace", $0] } ?? []) + ["--actor", "user"]
    }
}
