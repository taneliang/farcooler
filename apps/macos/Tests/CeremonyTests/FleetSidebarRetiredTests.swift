import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The old Fleet sidebar is gone (ov-178), and what only it offered has a
/// home elsewhere: the fleet's first-run and failure states in the detail,
/// a workspace's Show Charter and Wake the Agent When You Answer in the
/// orchestrator's header.
@MainActor
struct FleetSidebarRetiredTests {
    // MARK: - The detail says what the sidebar said before the fleet had anything

    /// Each was the sidebar's, and nothing else drew it: a first launch
    /// whose daemon wouldn't answer read "No Workspace Selected".
    @Test("The detail says loading, couldn't read, no repositories, no worktrees, then choose")
    func thePlaceholderFollowsTheFleet() {
        typealias P = FleetPlaceholder
        #expect(P.phase(hasWorktrees: false, localLoaded: false, localError: nil, hasRepositories: false) == .loading)
        #expect(
            P.phase(hasWorktrees: false, localLoaded: false, localError: "error: no daemon", hasRepositories: false)
                == .failed("error: no daemon"))
        #expect(P.phase(hasWorktrees: false, localLoaded: true, localError: nil, hasRepositories: false) == .noRepositories)
        #expect(P.phase(hasWorktrees: false, localLoaded: true, localError: nil, hasRepositories: true) == .noWorktrees)
        // A read that came back wins over an older failure.
        #expect(P.phase(hasWorktrees: false, localLoaded: true, localError: "stale", hasRepositories: true) == .noWorktrees)
        // Any runner's worktree is past all of it, this Mac's trouble or not.
        #expect(P.phase(hasWorktrees: true, localLoaded: false, localError: "down", hasRepositories: true) == .chooseWorkspace)
    }

    // MARK: - The orchestrator's header holds the workspace's own items

    /// Show Charter and Wake the Agent When You Answer were on the old
    /// sidebar's workspace row, and Wake was nowhere else. With no
    /// orchestrator, the header's menu was not drawn at all.
    @Test("With no orchestrator, the header still offers Show Charter and Wake the Agent When You Answer")
    func theHeaderOffersTheWorkspacesItemsWithoutASeat() {
        let charter = CharterAccess.unavailable("No charter yet.")
        #expect(
            ConversationHeader.menu(hasSeat: false, charter: charter, wakeOnAnswer: false)
                == [.showCharter, .wakeOnAnswer])
        #expect(
            ConversationHeader.menu(hasSeat: true, charter: charter, wakeOnAnswer: true)
                == [.replaceOrchestrator, .showCharter, .wakeOnAnswer])
        // A runner that can't wake anyone gets no switch, and starting one
        // is the column's placeholder's, never the header's.
        #expect(ConversationHeader.menu(hasSeat: false, charter: charter, wakeOnAnswer: nil) == [.showCharter])
        #expect(ConversationHeader.menu(hasSeat: false, charter: nil, wakeOnAnswer: nil).isEmpty)
    }
}
