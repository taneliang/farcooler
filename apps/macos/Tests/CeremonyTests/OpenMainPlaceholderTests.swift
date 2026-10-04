import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The empty detail's Open Main (ov-205): offered only where a Main
/// workspace exists to open.
@MainActor
struct OpenMainPlaceholderTests {
    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let other = "0198f2c0-0000-7000-8000-0000000000bb"

    private static func summary(_ id: String, _ name: String, isMain: Bool, repository: String) -> WorkspaceSummary {
        WorkspaceSummary(
            id: id, name: name, taskPrefix: String(name.prefix(3)).lowercased(), isMain: isMain,
            ordinal: isMain ? 0 : 1, repository: repository, orchestrator: nil)
    }

    private static func repository(_ id: String) -> (host: String, repository: Repository) {
        ("", Repository(id: id, short: "r", displayName: "overnight", remote: "", repositoryRootId: "root"))
    }

    private static func fleet(_ workspaces: [WorkspaceSummary]?) -> Fleet {
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [], branchPrefix: nil)
        if let workspaces { fleet.runnerWorkspaces[""] = workspaces }
        return fleet
    }

    @Test("Open Main is offered only when the repository has a Main workspace")
    func offeredOnlyWithMain() {
        let main = Self.summary("m", "Main", isMain: true, repository: Self.repo)
        let billing = Self.summary("b", "Billing", isMain: false, repository: Self.repo)
        let repos = [Self.repository(Self.repo)]
        #expect(FleetPlaceholder.mainToOpen(in: repos, fleet: Self.fleet([main, billing]))?.workspace.id == "m")
        // Workspaces listed, none of them Main: nothing to open.
        #expect(FleetPlaceholder.mainToOpen(in: repos, fleet: Self.fleet([billing])) == nil)
        // Another repository's Main isn't this one's.
        let elsewhere = Self.summary("x", "Main", isMain: true, repository: Self.other)
        #expect(FleetPlaceholder.mainToOpen(in: repos, fleet: Self.fleet([elsewhere])) == nil)
        // A runner without workspaces has one board per repository: its Main.
        #expect(FleetPlaceholder.mainToOpen(in: repos, fleet: Self.fleet(nil))?.workspace.id == Self.repo)
        // Several repositories: never a guess.
        let two = [Self.repository(Self.repo), Self.repository(Self.other)]
        #expect(FleetPlaceholder.mainToOpen(in: two, fleet: Self.fleet([main])) == nil)
        #expect(FleetPlaceholder.mainToOpen(in: [], fleet: Self.fleet([main])) == nil)
    }

    /// The owner on the first version, one five-line paragraph: "too many
    /// words. illustrations or bullets instead so that it's more scannable?"
    @Test("No Workspace Selected leads with purpose, then three short rows, never a paragraph")
    func explainerLeadsWithPurpose() {
        let copy = FleetPlaceholder.workspaceCopy
        #expect(copy.lede?.hasPrefix("Each workspace is one line of work") == true)
        #expect(copy.rows.map(\.symbol) == ["bubble.left", "checklist", "hand.raised"])
        EmptyStateCopyTests.expectScannable(copy)
    }
}
