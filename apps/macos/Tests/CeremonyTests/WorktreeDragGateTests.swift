import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Whether a sidebar row can be picked up and dragged into a new place.
///
/// The phone gated its reorder on the runner's `workspace_order` capability
/// and the Mac did not: every row on every runner was a drag source, and a
/// runner too old to keep an order answered `worktree reorder` with "unknown
/// method" while the row sprang back with nothing said. That failure is
/// invisible on the Mac this is developed on, whose runner is always current,
/// so the rule is pinned here rather than left to be noticed.
struct WorktreeDragGateTests {
    private static let current = DaemonBuild(
        version: "0.1.0+new", matches: true, platform: "macos",
        capabilities: ["workspaces", "terminals", "workspace_order"])
    private static let old = DaemonBuild(
        version: "0.1.0+old", matches: false, platform: "macos",
        capabilities: ["workspaces", "terminals", "watching"])

    @Test("A runner that keeps an order offers a drag")
    func aRunnerThatKeepsAnOrderOffersADrag() {
        #expect(WorktreeDrag.offersDrag(usable: true, runner: Self.current))
    }

    @Test("A runner too old to keep an order offers none")
    func aRunnerTooOldToKeepAnOrderOffersNone() {
        #expect(!WorktreeDrag.offersDrag(usable: true, runner: Self.old))
    }

    /// A daemon older than capabilities answers none at all, which `can(_:)`
    /// reads as the two features that existed then. Reordering is not one.
    @Test("A runner older than capabilities offers none")
    func aRunnerOlderThanCapabilitiesOffersNone() {
        let ancient = DaemonBuild(version: "0.0.1", matches: false, platform: "macos")
        #expect(!WorktreeDrag.offersDrag(usable: true, runner: ancient))
    }

    /// Refused, not guessed at: the build is read within a round trip of the
    /// link coming up, so the handle appears a moment late rather than a drag
    /// being offered that might go nowhere.
    @Test("A runner whose build has not been read offers none")
    func aRunnerWhoseBuildHasNotBeenReadOffersNone() {
        #expect(!WorktreeDrag.offersDrag(usable: true, runner: nil))
    }

    /// The capability does not outrank the link. A runner known to be
    /// unreachable offered no drag before this gate existed and must not
    /// start offering one because its last build read said it could.
    @Test("An unreachable runner offers none, whatever its build")
    func anUnreachableRunnerOffersNone() {
        #expect(!WorktreeDrag.offersDrag(usable: false, runner: Self.current))
    }

    /// Move to Workspace ▸ is the drag's menu equivalent (ruling 5), so it
    /// offers a workspace exactly when dropping the row on that workspace's
    /// row would move it there: never its own, never another repository's,
    /// never for the main checkout, and nothing on a runner that can't
    /// assign.
    @MainActor
    @Test("Move to Workspace offers exactly the targets the drag accepts")
    func moveToWorkspaceOffersExactlyTheTargetsTheDragAccepts() {
        let repo = "0198f2c0-0000-7000-8000-0000000000aa"
        let other = "0198f2c0-0000-7000-8000-0000000000bb"
        let workspaces = [
            WorkspaceSummary(id: "ws-main", name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: repo),
            WorkspaceSummary(id: "ws-bil", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: repo),
            WorkspaceSummary(id: "ws-ops", name: "Ops", taskPrefix: "ops", isMain: false, ordinal: 2, repository: repo),
            WorkspaceSummary(id: "ws-x", name: "Main", taskPrefix: "x", isMain: true, ordinal: 0, repository: other),
        ]
        func worktree(_ id: String, _ workspace: String?, main: Bool = false) -> Worktree {
            var w = Worktree(
                id: id, short: id, task: id, branch: "b", repository: "overnight", host: "", path: "/tmp/\(id)",
                state: "active", terminals: [], repositoryID: repo, workspace: workspace)
            w.is_main_checkout = main
            return w
        }
        let all = [worktree("lane", "ws-bil"), worktree("stray", nil), worktree("checkout", "ws-main", main: true)]
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: all, branchPrefix: nil)
        fleet.runnerWorkspaces[""] = workspaces
        for w in all {
            for assigns in [true, false] {
                let accepted = workspaces.filter {
                    ContentView.dropMeaning(w, onto: .workspace($0.id), in: fleet, assigns: assigns) != nil
                }.map(\.id)
                #expect(ContentView.moveTargets(for: w, in: fleet, assigns: assigns).map(\.id) == accepted, "\(w.id) \(assigns)")
            }
        }
        #expect(ContentView.moveTargets(for: all[0], in: fleet, assigns: true).map(\.id) == ["ws-main", "ws-ops"])
        #expect(ContentView.moveTargets(for: all[2], in: fleet, assigns: true).isEmpty)
    }
}
