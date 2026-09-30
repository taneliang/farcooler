import Foundation
import Testing

@testable import AgentKit

/// The shell's first screen, which was a spinner with no end in it.
///
/// Every case below is one a person reaches by owning a runner rather than by
/// doing anything unusual, and the one that matters most is the first: a
/// machine set up and not yet used answers with zero worktrees, which is a
/// fleet, and the old branch waited for a pane that was never coming.
struct ShellBringUpTests {
    private let somewhere = ShellPosition(worktree: 1, tab: 2)

    /// **The defect.** One runner, connected, no worktrees. `hasFleet` is true
    /// — it is set by the first successful refresh regardless of what came back
    /// — so nothing is on its way and nothing is going to arrive. A spinner
    /// here is permanent, and it is a spinner with no navigation bar and no
    /// host switcher under it.
    @Test func aConnectedRunnerWithNoWorktreesIsASentenceAndNotASpinner() {
        #expect(
            ShellBringUp.opening(seated: nil, worktrees: 0, reports: [.answered])
                == .noWorktrees)
    }

    /// A runner still dialing may yet bring worktrees, so the wait is real and
    /// has an end.
    @Test func aRunnerStillOnItsWayHoldsTheSpinner() {
        #expect(
            ShellBringUp.opening(seated: nil, worktrees: 0, reports: [.pending]) == .waiting)
        #expect(
            ShellBringUp.opening(
                seated: nil, worktrees: 0, reports: [.answered, .pending]) == .waiting)
    }

    /// A runner that FAILED does not hold it, and neither does one holding a
    /// fingerprint question. Both are waits with no end — and the second one
    /// most of all: the question is answered on the row, and a person cannot
    /// reach a row from behind a full-screen spinner.
    @Test func aStalledRunnerDoesNotHoldTheSpinner() {
        #expect(
            ShellBringUp.opening(seated: nil, worktrees: 0, reports: [.stalled])
                == .noWorktrees)
        #expect(
            ShellBringUp.opening(seated: nil, worktrees: 0, reports: [.answered, .stalled])
                == .noWorktrees)
    }

    /// The ordinary answer, and it is sticky: once the shell has been seeded
    /// nothing here can un-seed it. A fleet that empties under a mounted shell
    /// must leave the shell standing — tearing it down would throw away every
    /// pane in it, which is what `ShellPaneTrack` exists to prevent.
    @Test func aSeededShellStaysSeededThroughAnEmptyFleet() {
        #expect(
            ShellBringUp.opening(seated: somewhere, worktrees: 3, reports: [.answered])
                == .pane(somewhere))
        #expect(
            ShellBringUp.opening(seated: somewhere, worktrees: 0, reports: [.answered])
                == .pane(somewhere))
        #expect(
            ShellBringUp.opening(seated: somewhere, worktrees: 0, reports: [.stalled])
                == .pane(somewhere))
    }

    /// A fleet in hand with nothing seated yet is the one body pass between the
    /// worktrees arriving and the seeding that runs off them. A wait, and a
    /// short one — never the sentence, which would flash "No Worktrees" over a
    /// fleet that is already here.
    @Test func aFleetArrivedButNotYetSeededIsStillAWait() {
        #expect(
            ShellBringUp.opening(seated: nil, worktrees: 4, reports: [.answered]) == .waiting)
    }

    /// No runners at all is not a wait either. Nobody is coming.
    @Test func noRunnersIsNotAWait() {
        #expect(ShellBringUp.opening(seated: nil, worktrees: 0, reports: []) == .noWorktrees)
    }
}
