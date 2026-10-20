import Foundation
import Testing

@testable import AgentKit

/// A worktree's panes as positions in a fleet, and the phone's stack,
/// checked where a screen is not needed: the iOS app has no unit test target.
/// The shell's bar and its gestures are gone (ov-617), and their arithmetic
/// with them.
///
/// The fixtures are shaped for the questions rather than for realism. The one
/// that matters is `crossing`: a fleet whose worktrees have DIFFERENT numbers
/// of tabs, because a fleet where every worktree has two tabs cannot tell
/// "the previous worktree's last tab" apart from "the previous worktree's
/// tab 1".
struct ShellNavigationTests {
    /// Three worktrees, 2 / 3 / 1 tabs. Every index in this fleet is
    /// distinguishable from every other by number alone.
    private static func crossing() -> ShellFleet {
        ShellFleet(worktrees: [
            ShellWorktree(
                id: "a", name: "alpha",
                tabs: [tab("a0", .working), tab("a1", .working)]),
            ShellWorktree(
                id: "b", name: "beta",
                tabs: [tab("b0", .working), tab("b1", .working), tab("b2", .working)]),
            ShellWorktree(id: "c", name: "gamma", tabs: [tab("c0", .working)]),
        ])
    }

    /// The shorthand these tests are written in.
    ///
    /// **A TEST vocabulary, and deliberately no longer the app's.** `ShellMark`
    /// was an enum of exactly these names in the app itself, and it is retired
    /// — `ShellTab.mark` says why. What the navigation and precedence tests
    /// need is still a one-word way to say "a worktree with a blocked tab in
    /// it", and spelling three axes at forty call sites would bury the thing
    /// each test is actually about. So the shorthand stays here, where it is
    /// scenery, and the app holds a `GlanceMark`.
    ///
    /// `done` and `idle` are new and are not decoration: they are the two
    /// states the old enum could not say, and the tests below that matter most
    /// are the ones that tell them from `needsYou` and `working`.
    enum TabState {
        case needsYou
        case unreadDiff
        case working
        case idle
        case done
        case stale
    }

    private static func tab(_ id: String, _ state: TabState) -> ShellTab {
        switch state {
        case .needsYou:
            return ShellTab(
                id: id, title: id, mark: GlanceMark(attention: .needsYou, core: .atAPrompt),
                wantsAttention: true)
        case .done:
            return ShellTab(
                id: id, title: id, mark: GlanceMark(attention: .toReview, core: .atAPrompt),
                wantsAttention: true)
        // A diff, which states no core because it has no agent behind it.
        case .unreadDiff:
            return ShellTab(
                id: id, title: id, mark: GlanceMark(attention: .toReview, core: nil))
        case .working:
            return ShellTab(
                id: id, title: id, mark: GlanceMark(attention: .quiet, core: .producing))
        case .idle:
            return ShellTab(
                id: id, title: id, mark: GlanceMark(attention: .quiet, core: .atAPrompt))
        case .stale:
            return ShellTab(
                id: id, title: id,
                mark: GlanceMark(attention: .quiet, core: nil, link: .broken))
        }
    }

    /// A remembered tab that has since gone is not honoured as an index.
    ///
    /// Terminals exit constantly, and a `resume` resolved against the fleet
    /// the caller was holding can be one poll out of date. Out of range lands
    /// on the first tab rather than trapping or landing on the last.
    @Test func aRememberedTabThatIsGoneFallsBackToTheFirst() {
        let worktree = ShellWorktree(
            id: "b", name: "beta", resume: 5,
            tabs: [Self.tab("b0", .working), Self.tab("b1", .working)])
        #expect(worktree.resumeTab == 0)
        #expect(ShellWorktree(id: "e", name: "e", resume: 0, tabs: []).resumeTab == 0)
        #expect(
            ShellWorktree(id: "n", name: "n", tabs: [Self.tab("n0", .working)]).resumeTab == 0)
    }

    // MARK: - Finding a pane that is already mounted

    /// **A retained pane is found by ID, wherever the fleet has moved it.**
    ///
    /// This is what lets `ShellPaneTrack` mount a pane once and place it by
    /// slot: a worktree that gains a terminal renumbers every index after it,
    /// and a pane looked up by cached index would silently start naming a
    /// different tab. Nil is the prune rule — a pane whose id no longer
    /// resolves is a pane for something the runner has forgotten.
    @Test func aTabIsFoundByIdAcrossTheWholeFleet() {
        let fleet = Self.crossing()
        #expect(fleet.position(ofTab: "b2") == ShellPosition(worktree: 1, tab: 2))
        #expect(fleet.position(ofTab: "c0") == ShellPosition(worktree: 2, tab: 0))
        #expect(fleet.position(ofTab: "gone") == nil)
        #expect(fleet.tab(at: ShellPosition(worktree: 1, tab: 2))?.id == "b2")
        #expect(fleet.tab(at: ShellPosition(worktree: 9, tab: 0)) == nil)
        #expect(fleet.tab(at: ShellPosition(worktree: 2, tab: 4)) == nil)
    }

    /// The same tab, after a terminal is inserted before it: a new index, the
    /// same identity. The pane must move, not be rebuilt.
    @Test func insertingATabMovesTheOnesAfterItRatherThanRenamingThem() {
        var fleet = Self.crossing()
        #expect(fleet.position(ofTab: "b1") == ShellPosition(worktree: 1, tab: 1))
        fleet.worktrees[1].tabs.insert(Self.tab("b-new", .working), at: 0)
        #expect(fleet.position(ofTab: "b1") == ShellPosition(worktree: 1, tab: 2))
        #expect(fleet.position(ofTab: "b-new") == ShellPosition(worktree: 1, tab: 0))
    }

    // MARK: - Where the shell sits when the fleet is re-merged

    /// **The bug: a later-arriving runner re-indexes the pane on screen.**
    ///
    /// Two runners on a cold launch. The first answers, the shell opens on its
    /// second worktree, and seconds later the second runner answers — its
    /// worktrees go in ahead, in runner-list order, and `(worktree: 1)` is now
    /// somebody else's checkout. The value did not move, so nothing downstream
    /// can see that everything under it did.
    @Test func aRunnerArrivingAheadDoesNotTakeTheShellWithIt() {
        let before = Self.crossing()
        let at = ShellPosition(worktree: 1, tab: 2)
        #expect(before.tab(at: at)?.id == "b2")

        var after = before
        after.worktrees.insert(
            ShellWorktree(id: "z", name: "zulu", tabs: [Self.tab("z0", .working)]), at: 0)

        // The index alone is now a different runner's worktree.
        #expect(after.tab(at: at)?.id != "b2")
        // Anchored to the tab, the shell stays on the pane somebody is reading.
        #expect(after.reseat(at, holding: "b2") == ShellPosition(worktree: 2, tab: 2))
        #expect(after.tab(at: after.reseat(at, holding: "b2"))?.id == "b2")
    }

    /// **A fleet that arrives after an empty one lands on its first worktree.**
    ///
    /// A one-runner-at-a-time switch: A is retired, the fleet empties, B
    /// answers. The shell's index is A's, and clamping it would stand the shell
    /// on whatever B keeps at that number.
    @Test func aFleetArrivingAfterAnEmptyOneLandsOnItsFirstWorktree() {
        let empty = ShellFleet(worktrees: [])
        let fleet = Self.crossing()
        let at = ShellPosition(worktree: 1, tab: 2)
        let landed = fleet.reseat(at, holding: "gone", after: empty)
        #expect(landed.worktree == 0)
        #expect(landed.tab == fleet.worktrees[0].resumeTab)
        // Across an ordinary poll, the anchor-less answer is still the clamp.
        #expect(fleet.reseat(at, holding: "gone", after: fleet) == fleet.clamping(at))
        // And an anchor that IS here wins either way.
        #expect(fleet.reseat(at, holding: "b2", after: empty) == at)
    }

    /// A tab inserted before this one inside the same worktree moves it too.
    @Test func aTabInsertedAheadMovesTheAnchoredPosition() {
        var fleet = Self.crossing()
        fleet.worktrees[1].tabs.insert(Self.tab("b-new", .working), at: 0)
        #expect(
            fleet.reseat(ShellPosition(worktree: 1, tab: 1), holding: "b1")
                == ShellPosition(worktree: 1, tab: 2))
    }

    /// Nothing moved, so nothing moves. The anchored position is the position.
    @Test func anUnchangedFleetSeatsTheShellWhereItAlreadyIs() {
        let fleet = Self.crossing()
        let at = ShellPosition(worktree: 1, tab: 2)
        #expect(fleet.reseat(at, holding: "b2") == at)
    }

    /// The tab is gone — a terminal exited, a worktree was removed — and the
    /// answer is the NEAREST position rather than the front of the fleet.
    /// Somebody reading `beta` should not be moved to `alpha` because a pane
    /// they were not looking at went away.
    @Test func aVanishedTabClampsRatherThanGoingBackToTheStart() {
        var fleet = Self.crossing()
        fleet.worktrees[1].tabs.removeLast()
        #expect(
            fleet.reseat(ShellPosition(worktree: 1, tab: 2), holding: "b2")
                == ShellPosition(worktree: 1, tab: 1))
    }

    /// A whole worktree gone clamps to the last one there is, not to nothing.
    /// The TAB clamps within it too, and independently: `beta` is three tabs
    /// long, so tab 0 stays tab 0 rather than being carried to the end.
    @Test func aVanishedWorktreeClampsToTheEndOfTheFleet() {
        var fleet = Self.crossing()
        fleet.worktrees.removeLast()
        #expect(
            fleet.reseat(ShellPosition(worktree: 2, tab: 0), holding: "c0")
                == ShellPosition(worktree: 1, tab: 0))
        #expect(
            fleet.reseat(ShellPosition(worktree: 2, tab: 7), holding: "c0")
                == ShellPosition(worktree: 1, tab: 2))
    }

    /// A fleet with nothing in it hands the position back untouched. There is
    /// nothing to clamp to, and a fleet that empties under a finger is ordinary
    /// — the same reason `tabCount(ofWorktree:)` answers 0 rather than
    /// trapping.
    @Test func anEmptyFleetLeavesThePositionAlone() {
        let fleet = ShellFleet(worktrees: [])
        let at = ShellPosition(worktree: 1, tab: 2)
        #expect(fleet.reseat(at, holding: "b2") == at)
    }

    /// A shell that has not come to rest on anything yet clamps, the same as
    /// one whose tab has gone.
    @Test func noAnchorClamps() {
        let fleet = Self.crossing()
        #expect(
            fleet.reseat(ShellPosition(worktree: 9, tab: 9), holding: nil)
                == ShellPosition(worktree: 2, tab: 0))
    }

    // MARK: - A runner that is not answering

    /// **A reconnecting runner's tabs say what they can and no more.** Its
    /// fleet is the last one read before the link went, kept so the screen
    /// doesn't move, and every claim about now in it becomes "can't say":
    /// the working and idle tabs, and the quiet Changes. Blocked, done and an
    /// unread diff hold — an agent that stopped for you is still stopped for
    /// you.
    ///
    /// Mutation: `said(answering:)` returning `self` whatever `answering` is.
    /// Red on every quiet tab.
    @Test func aRunnerThatIsntAnsweringCantSayWhatItsAgentsAreDoing() {
        let worktree = ShellWorktree(
            id: "w", name: "feat/queue",
            tabs: [
                Self.tab("diff", .unreadDiff), Self.tab("blocked", .needsYou),
                Self.tab("done", .done), Self.tab("working", .working),
                Self.tab("idle", .idle),
                ShellTab(id: "quiet-diff", title: "Diff", mark: GlanceMark(attention: .quiet, core: nil)),
            ])

        #expect(worktree.said(answering: true) == worktree, "answering, nothing changes")

        let lost = worktree.said(answering: false)
        #expect(lost.tabs.map(\.mark) == [
            worktree.tabs[0].mark, worktree.tabs[1].mark, worktree.tabs[2].mark,
            .unsaid, .unsaid, .unsaid,
        ])
        #expect(lost.tabs.map(\.wantsAttention) == worktree.tabs.map(\.wantsAttention))
    }

    /// A fleet that emptied under the screen is survivable.
    @Test func aPositionTheFleetNoLongerHasIsSurvivable() {
        let fleet = ShellFleet(worktrees: [])
        #expect(fleet.tabCount(ofWorktree: 4) == 0)
        #expect(fleet.tab(at: ShellPosition(worktree: 4, tab: 9)) == nil)
        #expect(fleet.first == nil)
    }

    // MARK: - The phone's stack (ov-55 4A)

    private static let billing = PhoneWorkspace(runner: "studio", workspace: "ws-billing")

    /// **The front door is the inbox.** With anything in it, a launch pushes
    /// nothing over Needs You, whatever workspace was open last.
    @Test("Launch lands on Needs You when it has items")
    func launchLandsOnNeedsYouWhenItHasItems() {
        #expect(PhoneLaunch.stack(itemCount: 1, last: Self.billing, exists: { _ in true }) == [])
        #expect(PhoneLaunch.stack(itemCount: 3, last: nil, exists: { _ in true }) == [])
        // Not before every runner has said: a second runner may be about to
        // fill the inbox the first left empty.
        #expect(!PhoneLaunch.canDecide([.read, .waiting]))
        #expect(PhoneLaunch.canDecide([.read, .unreachable]))
        #expect(!PhoneLaunch.canDecide([]))
    }

    /// **An empty inbox is a screen with nothing to do**, so the last
    /// workspace is pushed over it, and Back still reaches Needs You. A
    /// workspace that's gone since isn't pushed at all.
    @Test("With nothing waiting, launch pushes the last workspace over Needs You")
    func withNothingWaitingLaunchPushesTheLastWorkspace() {
        #expect(
            PhoneLaunch.stack(itemCount: 0, last: Self.billing, exists: { $0 == Self.billing })
                == [.workspace(Self.billing)])
        #expect(PhoneLaunch.stack(itemCount: 0, last: Self.billing, exists: { _ in false }) == [])
        #expect(PhoneLaunch.stack(itemCount: 0, last: nil, exists: { _ in true }) == [])
        // Kept as one string, and read back as the same workspace.
        #expect(PhoneWorkspace(stored: Self.billing.stored) == Self.billing)
        #expect(PhoneWorkspace(stored: "no-bar") == nil)
    }
}
