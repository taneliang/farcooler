import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Whether a sidebar row can be picked up and dragged into a new place.
///
/// The phone gated its reorder on the runner's `workspace_order` capability
/// and the Mac did not: every row on every runner was a drag source, and a
/// runner too old to keep an order answered `workspace reorder` with "unknown
/// method" while the row sprang back with nothing said. That failure is
/// invisible on the Mac this is developed on, whose runner is always current,
/// so the rule is pinned here rather than left to be noticed.
struct WorkspaceDragGateTests {
    private static let current = DaemonBuild(
        version: "0.1.0+new", matches: true, platform: "macos",
        capabilities: ["workspaces", "terminals", "workspace_order"])
    private static let old = DaemonBuild(
        version: "0.1.0+old", matches: false, platform: "macos",
        capabilities: ["workspaces", "terminals", "watching"])

    @Test("A runner that keeps an order offers a drag")
    func aRunnerThatKeepsAnOrderOffersADrag() {
        #expect(WorkspaceDrag.offersDrag(usable: true, runner: Self.current))
    }

    @Test("A runner too old to keep an order offers none")
    func aRunnerTooOldToKeepAnOrderOffersNone() {
        #expect(!WorkspaceDrag.offersDrag(usable: true, runner: Self.old))
    }

    /// A daemon older than capabilities answers none at all, which `can(_:)`
    /// reads as the two features that existed then. Reordering is not one.
    @Test("A runner older than capabilities offers none")
    func aRunnerOlderThanCapabilitiesOffersNone() {
        let ancient = DaemonBuild(version: "0.0.1", matches: false, platform: "macos")
        #expect(!WorkspaceDrag.offersDrag(usable: true, runner: ancient))
    }

    /// Refused, not guessed at: the build is read within a round trip of the
    /// link coming up, so the handle appears a moment late rather than a drag
    /// being offered that might go nowhere.
    @Test("A runner whose build has not been read offers none")
    func aRunnerWhoseBuildHasNotBeenReadOffersNone() {
        #expect(!WorkspaceDrag.offersDrag(usable: true, runner: nil))
    }

    /// The capability does not outrank the link. A runner known to be
    /// unreachable offered no drag before this gate existed and must not
    /// start offering one because its last build read said it could.
    @Test("An unreachable runner offers none, whatever its build")
    func anUnreachableRunnerOffersNone() {
        #expect(!WorkspaceDrag.offersDrag(usable: false, runner: Self.current))
    }
}
