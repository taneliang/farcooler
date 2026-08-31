import Foundation
import Testing

@testable import AgentKit

/// The navigation shell's arithmetic, checked where a screen is not needed.
///
/// This suite exists because the iOS app has no unit test target: the only
/// tests that target generates are UI tests, and a UI test needs a booted
/// simulator, a launch, and a real swipe to ask one question. Every rule below
/// is one a person would otherwise have to discover by swiping — and three of
/// them are rules where being wrong looks perfectly normal on a screenshot.
///
/// The fixtures are shaped for the questions rather than for realism. The one
/// that matters is `crossing`: a fleet whose workspaces have DIFFERENT numbers
/// of tabs, because a fleet where every workspace has two tabs cannot tell
/// "the previous workspace's last tab" apart from "the previous workspace's
/// tab 1".
struct ShellNavigationTests {
    /// Three workspaces, 2 / 3 / 1 tabs. Every index in this fleet is
    /// distinguishable from every other by number alone.
    private static func crossing() -> ShellFleet {
        ShellFleet(workspaces: [
            ShellWorkspace(
                id: "a", name: "alpha",
                tabs: [tab("a0", .working), tab("a1", .working)]),
            ShellWorkspace(
                id: "b", name: "beta",
                tabs: [tab("b0", .working), tab("b1", .working), tab("b2", .working)]),
            ShellWorkspace(id: "c", name: "gamma", tabs: [tab("c0", .working)]),
        ])
    }

    private static func tab(_ id: String, _ mark: ShellMark) -> ShellTab {
        ShellTab(id: id, title: id, mark: mark)
    }

    // MARK: - The flat sequence

    /// The rule the whole content gesture is: walking backward off the FIRST
    /// tab of a workspace lands on the PREVIOUS workspace's LAST tab.
    ///
    /// Landing on its tab 0 instead would be the same bug in both directions —
    /// swipe back then forward and you are somewhere you did not start — and
    /// it is invisible in a fleet where every workspace has the same number of
    /// tabs, which is why `beta` has three and `alpha` has two.
    @Test func backwardOffAWorkspaceLandsOnThePreviousOnesLastTab() {
        let fleet = Self.crossing()
        let step = fleet.step(
            from: ShellPosition(workspace: 1, tab: 0), .previous, along: .content)
        #expect(step?.position == ShellPosition(workspace: 0, tab: 1))
        #expect(step?.crossesWorkspace == true)
    }

    /// And forward off the last tab lands on the next workspace's FIRST.
    @Test func forwardOffAWorkspaceLandsOnTheNextOnesFirstTab() {
        let fleet = Self.crossing()
        let step = fleet.step(
            from: ShellPosition(workspace: 0, tab: 1), .next, along: .content)
        #expect(step?.position == ShellPosition(workspace: 1, tab: 0))
        #expect(step?.crossesWorkspace == true)
    }

    /// Within a workspace nothing is crossed, and the flag says so — which is
    /// what keeps the bar still for a swipe that does not change workspace.
    @Test func aStepInsideAWorkspaceIsNotACrossing() {
        let fleet = Self.crossing()
        let step = fleet.step(
            from: ShellPosition(workspace: 1, tab: 0), .next, along: .content)
        #expect(step?.position == ShellPosition(workspace: 1, tab: 1))
        #expect(step?.crossesWorkspace == false)
    }

    /// Walking the whole fleet forward from the very first tab visits every
    /// tab of every workspace in order, exactly once, and then stops.
    ///
    /// The sequence is asserted as a whole rather than one step at a time
    /// because "flat" is a property of the walk, not of any single step: a
    /// step function that skipped one tab or repeated one at each boundary
    /// passes every individual assertion above.
    @Test func theSequenceIsFlatAcrossTheWholeFleet() {
        let fleet = Self.crossing()
        var seen: [ShellPosition] = []
        var at = ShellPosition(workspace: 0, tab: 0)
        seen.append(at)
        while let step = fleet.step(from: at, .next, along: .content) {
            at = step.position
            seen.append(at)
            #expect(seen.count <= 10, "the sequence did not terminate")
        }
        #expect(
            seen == [
                ShellPosition(workspace: 0, tab: 0), ShellPosition(workspace: 0, tab: 1),
                ShellPosition(workspace: 1, tab: 0), ShellPosition(workspace: 1, tab: 1),
                ShellPosition(workspace: 1, tab: 2), ShellPosition(workspace: 2, tab: 0),
            ])
        // And backward is the same walk in reverse, which is the property that
        // makes a swipe undoable by the opposite swipe.
        var back: [ShellPosition] = [at]
        while let step = fleet.step(from: at, .previous, along: .content) {
            at = step.position
            back.append(at)
            #expect(back.count <= 10, "the backward sequence did not terminate")
        }
        #expect(back == seen.reversed())
    }

    /// A workspace with no tabs is stepped OVER, not landed on: a swipe that
    /// arrived on nothing could not be swiped out of the same way.
    @Test func anEmptyWorkspaceIsSteppedOver() {
        let fleet = ShellFleet(workspaces: [
            ShellWorkspace(id: "a", name: "alpha", tabs: [Self.tab("a0", .working)]),
            ShellWorkspace(id: "empty", name: "empty", tabs: []),
            ShellWorkspace(id: "c", name: "gamma", tabs: [Self.tab("c0", .working)]),
        ])
        #expect(
            fleet.step(from: ShellPosition(workspace: 0, tab: 0), .next, along: .content)?.position
                == ShellPosition(workspace: 2, tab: 0))
        #expect(
            fleet.step(from: ShellPosition(workspace: 2, tab: 0), .previous, along: .content)?
                .position == ShellPosition(workspace: 0, tab: 0))
    }

    /// The bar walks WORKSPACES, and — where nobody has ever chosen a tab in
    /// one — at its first tab, in both directions.
    @Test func theBarStepsWholeWorkspacesAtTheirFirstTab() {
        let fleet = Self.crossing()
        #expect(
            fleet.step(from: ShellPosition(workspace: 1, tab: 2), .next, along: .bar)?.position
                == ShellPosition(workspace: 2, tab: 0))
        #expect(
            fleet.step(from: ShellPosition(workspace: 1, tab: 2), .previous, along: .bar)?.position
                == ShellPosition(workspace: 0, tab: 0))
        #expect(fleet.step(from: ShellPosition(workspace: 0, tab: 0), .previous, along: .bar) == nil)
        #expect(fleet.step(from: ShellPosition(workspace: 2, tab: 0), .next, along: .bar) == nil)
    }

    // MARK: - Reopening a workspace where you left it

    /// **The bar swipe lands on the tab you last had open, not on tab 0.**
    ///
    /// Crossing to a workspace by name is going back to a place, and a place
    /// you come back to should be where you left it — the same argument
    /// `docs/jobs-to-be-done.md` F4 makes about the app surviving being put
    /// down every ninety seconds. Landing on tab 0 is the app half-remembering.
    @Test func theBarStepLandsOnTheRememberedTab() {
        let fleet = ShellFleet(workspaces: [
            ShellWorkspace(id: "a", name: "alpha", tabs: [Self.tab("a0", .working)]),
            ShellWorkspace(
                id: "b", name: "beta", resume: 2,
                tabs: [Self.tab("b0", .working), Self.tab("b1", .working), Self.tab("b2", .working)]
            ),
        ])
        #expect(
            fleet.step(from: ShellPosition(workspace: 0, tab: 0), .next, along: .bar)?.position
                == ShellPosition(workspace: 1, tab: 2))
    }

    /// **And the content swipe does not.** It walks one flat sequence and has
    /// to stay literal: a sequence that jumped to a remembered tab would drift
    /// — swipe forward then back and you are somewhere you have never been —
    /// and it would silently skip every tab between here and the memory.
    ///
    /// The same fleet as the test above, so what is being checked is the
    /// difference between the two tracks and nothing else.
    @Test func theContentStepIgnoresTheRememberedTab() {
        let fleet = ShellFleet(workspaces: [
            ShellWorkspace(id: "a", name: "alpha", tabs: [Self.tab("a0", .working)]),
            ShellWorkspace(
                id: "b", name: "beta", resume: 2,
                tabs: [Self.tab("b0", .working), Self.tab("b1", .working), Self.tab("b2", .working)]
            ),
        ])
        #expect(
            fleet.step(from: ShellPosition(workspace: 0, tab: 0), .next, along: .content)?.position
                == ShellPosition(workspace: 1, tab: 0))
        // And backward off `beta` still lands on `alpha`'s LAST tab rather
        // than on anything remembered — `alpha` has no memory, but the rule is
        // the sequence's, not the absence of one.
        #expect(
            fleet.step(from: ShellPosition(workspace: 1, tab: 0), .previous, along: .content)?
                .position == ShellPosition(workspace: 0, tab: 0))
    }

    /// A remembered tab that has since gone is not honoured as an index.
    ///
    /// Terminals exit constantly, and a `resume` resolved against the fleet
    /// the caller was holding can be one poll out of date. Out of range lands
    /// on the first tab rather than trapping or landing on the last.
    @Test func aRememberedTabThatIsGoneFallsBackToTheFirst() {
        let workspace = ShellWorkspace(
            id: "b", name: "beta", resume: 5,
            tabs: [Self.tab("b0", .working), Self.tab("b1", .working)])
        #expect(workspace.resumeTab == 0)
        #expect(ShellWorkspace(id: "e", name: "e", resume: 0, tabs: []).resumeTab == 0)
        #expect(
            ShellWorkspace(id: "n", name: "n", tabs: [Self.tab("n0", .working)]).resumeTab == 0)
    }

    // MARK: - Finding a pane that is already mounted

    /// **A retained pane is found by ID, wherever the fleet has moved it.**
    ///
    /// This is what lets `ShellPaneTrack` mount a pane once and place it by
    /// slot: a workspace that gains a terminal renumbers every index after it,
    /// and a pane looked up by cached index would silently start naming a
    /// different tab. Nil is the prune rule — a pane whose id no longer
    /// resolves is a pane for something the runner has forgotten.
    @Test func aTabIsFoundByIdAcrossTheWholeFleet() {
        let fleet = Self.crossing()
        #expect(fleet.position(ofTab: "b2") == ShellPosition(workspace: 1, tab: 2))
        #expect(fleet.position(ofTab: "c0") == ShellPosition(workspace: 2, tab: 0))
        #expect(fleet.position(ofTab: "gone") == nil)
        #expect(fleet.tab(at: ShellPosition(workspace: 1, tab: 2))?.id == "b2")
        #expect(fleet.tab(at: ShellPosition(workspace: 9, tab: 0)) == nil)
        #expect(fleet.tab(at: ShellPosition(workspace: 2, tab: 4)) == nil)
    }

    /// The same tab, after a terminal is inserted before it: a new index, the
    /// same identity. The pane must move, not be rebuilt.
    @Test func insertingATabMovesTheOnesAfterItRatherThanRenamingThem() {
        var fleet = Self.crossing()
        #expect(fleet.position(ofTab: "b1") == ShellPosition(workspace: 1, tab: 1))
        fleet.workspaces[1].tabs.insert(Self.tab("b-new", .working), at: 0)
        #expect(fleet.position(ofTab: "b1") == ShellPosition(workspace: 1, tab: 2))
        #expect(fleet.position(ofTab: "b-new") == ShellPosition(workspace: 1, tab: 0))
    }

    // MARK: - Rubber banding

    /// The rubber band engages at the two true ends of the FLEET and nowhere
    /// else — in particular not at a workspace boundary, which is the mistake
    /// that would make the fleet feel like a list of separate lists.
    @Test func rubberBandingIsOnlyAtTheTwoEndsOfTheFleet() {
        let fleet = Self.crossing()
        let first = ShellPosition(workspace: 0, tab: 0)
        let last = ShellPosition(workspace: 2, tab: 0)
        #expect(fleet.rubberBands(at: first, .previous, along: .content))
        #expect(fleet.rubberBands(at: last, .next, along: .content))
        #expect(!fleet.rubberBands(at: first, .next, along: .content))
        #expect(!fleet.rubberBands(at: last, .previous, along: .content))
        // The boundaries in between: last tab of `alpha`, first tab of `beta`.
        #expect(
            !fleet.rubberBands(at: ShellPosition(workspace: 0, tab: 1), .next, along: .content))
        #expect(
            !fleet.rubberBands(at: ShellPosition(workspace: 1, tab: 0), .previous, along: .content))
    }

    /// From the BAR the ends are the ends of the workspace list, so the first
    /// workspace's last tab still rubber-bands forward — the bar is not
    /// walking tabs.
    @Test func theBarsEndsAreTheWorkspaceListsEnds() {
        let fleet = Self.crossing()
        #expect(fleet.rubberBands(at: ShellPosition(workspace: 0, tab: 0), .previous, along: .bar))
        #expect(!fleet.rubberBands(at: ShellPosition(workspace: 0, tab: 1), .next, along: .bar))
        #expect(fleet.rubberBands(at: ShellPosition(workspace: 2, tab: 0), .next, along: .bar))
    }

    @Test func aRubberBandedDragMovesAThirdAsFar() {
        #expect(ShellGesture.translation(dx: 100, rubberBanding: false) == 100)
        #expect(ShellGesture.translation(dx: 100, rubberBanding: true) == 34)
    }

    // MARK: - The axis lock

    /// Nothing under 6 points has an axis at all, which is what stops a tap
    /// from being a tiny drag.
    @Test func theAxisIsUndecidedUnderSixPoints() {
        #expect(ShellGesture.axis(dx: 0, up: 0) == nil)
        #expect(ShellGesture.axis(dx: 5.9, up: 0) == nil)
        #expect(ShellGesture.axis(dx: 0, up: 5.9) == nil)
        #expect(ShellGesture.axis(dx: -5.9, up: -5.9) == nil)
        #expect(ShellGesture.axis(dx: 6.1, up: 0) == .horizontal)
        #expect(ShellGesture.axis(dx: 0, up: 6.1) == .vertical)
    }

    /// `abs(dx) > dy`, up-positive — so a DOWNWARD drag is horizontal, and
    /// then does nothing because its `dx` is nowhere near the commit
    /// threshold. Locking it vertical instead would lock it to the one axis on
    /// which the bar has nothing to offer.
    @Test func aDownwardDragIsHorizontalAndThereforeHarmless() {
        #expect(ShellGesture.axis(dx: 0, up: -40) == .horizontal)
        let fleet = Self.crossing()
        #expect(
            fleet.barRelease(
                axis: .horizontal, dx: 0, up: 0, at: ShellPosition(workspace: 1, tab: 0))
                == .springBack)
    }

    // MARK: - The column

    /// One row selected per `rowHeight` of travel, walking UPWARD from the row
    /// nearest the bar, and no selection at all below `openMin`.
    ///
    /// The menu reads top to bottom — tab 0 at the top, the same end the
    /// ribbon draws its first mark at — so the row the first `rowHeight` of
    /// lift puts under the thumb is the LAST tab, and a lift that fills the
    /// column has walked all the way up to tab 0. Every number here is written
    /// as a multiple of `ShellMetrics.rowHeight` rather than as the 44 it
    /// happens to be: the constant moved from 34 to 44 once already.
    @Test func theColumnSelectsOneRowPerRowHeightOfTravelWalkingUpward() {
        #expect(ShellGesture.columnSelection(up: 0, tabCount: 3) == nil)
        #expect(ShellGesture.columnSelection(up: ShellMetrics.openMin - 0.1, tabCount: 3) == nil)
        #expect(
            ShellGesture.columnSelection(up: ShellMetrics.openMin, tabCount: 3) == 2,
            "the first points of lift select the row nearest the bar, which is the LAST tab")
        #expect(ShellGesture.columnSelection(up: ShellMetrics.rowHeight - 1, tabCount: 3) == 2)
        #expect(ShellGesture.columnSelection(up: ShellMetrics.rowHeight + 1, tabCount: 3) == 1)
        #expect(ShellGesture.columnSelection(up: ShellMetrics.rowHeight * 2 - 1, tabCount: 3) == 1)
        #expect(
            ShellGesture.columnSelection(up: 400, tabCount: 3) == 0,
            "capped at the TOP row, which is tab 0")
        #expect(ShellGesture.columnSelection(up: 400, tabCount: 0) == nil)
    }

    /// Pinned open is a SEPARATE input from the lift, and outranks it. This is
    /// the separation the prototype keeps `colOpen` distinct from `dragY` for:
    /// a column that derived its height from both would close itself the
    /// instant a tap-opened column was touched again.
    @Test func aPinnedColumnIsFullHeightWhateverTheLiftSays() {
        #expect(ShellGesture.columnHeight(up: 0, tabCount: 3, pinned: true) == ShellMetrics.rowHeight * 3)
        #expect(ShellGesture.columnHeight(up: 0, tabCount: 3, pinned: false) == 0)
        #expect(
            ShellGesture.columnHeight(up: -80, tabCount: 3, pinned: false) == 0,
            "a downward drag does not give the column a negative height")
    }

    /// Whole, or nothing — never a row at a time.
    ///
    /// The column used to grow with the lift, so a person choosing between
    /// four tabs could see one of them until they had dragged past three. The
    /// options have to be on screen to be options.
    @Test func theColumnOpensWholeRatherThanARowAtATime() {
        #expect(
            ShellGesture.columnHeight(up: ShellMetrics.openMin - 1, tabCount: 4, pinned: false) == 0,
            "below the threshold it is shut, and a release there abandons")
        #expect(
            ShellGesture.columnHeight(up: ShellMetrics.openMin, tabCount: 4, pinned: false) == ShellMetrics.rowHeight * 4,
            "at the threshold every row is on screen at once")
        #expect(
            ShellGesture.columnHeight(up: 500, tabCount: 4, pinned: false) == ShellMetrics.rowHeight * 4,
            "and dragging further never grows it past its rows")
    }

    /// The lift still chooses, even though it no longer reveals.
    ///
    /// This is the half of the original behaviour worth keeping: one
    /// continuous drag still walks the tabs and carries on into the overview.
    @Test func theSelectionStillFollowsTheFingerThroughAnOpenColumn() {
        #expect(ShellGesture.columnSelection(up: 10, tabCount: 4) == nil)
        #expect(ShellGesture.columnSelection(up: 20, tabCount: 4) == 3)
        #expect(ShellGesture.columnSelection(up: ShellMetrics.rowHeight + 6, tabCount: 4) == 2)
        #expect(ShellGesture.columnSelection(up: ShellMetrics.rowHeight * 2 + 6, tabCount: 4) == 1)
        #expect(
            ShellGesture.columnSelection(up: 500, tabCount: 4) == 0,
            "and it never selects past the TOP row, however far the finger goes")
    }

    // MARK: - The overview

    /// The overview starts where the column runs out, so it is always the same
    /// gesture — keep going — whether the workspace has one tab or nine.
    @Test func theOverviewBeginsWhereTheColumnRunsOut() {
        #expect(ShellGesture.overviewProgress(up: 102, tabCount: 3) == 0)
        #expect(ShellGesture.overviewProgress(up: ShellMetrics.rowHeight * 3 + ShellMetrics.overRun / 2, tabCount: 3) == 0.5)
        #expect(ShellGesture.overviewProgress(up: ShellMetrics.rowHeight * 3 + ShellMetrics.overRun, tabCount: 3) == 1)
        #expect(ShellGesture.overviewProgress(up: 400, tabCount: 3) == 1)
        #expect(ShellGesture.overviewProgress(up: 0, tabCount: 3) == 0)
        // A one-tab workspace reaches it 68 points sooner, and that is the
        // point: the distance is measured from the end of the column.
        #expect(ShellGesture.overviewProgress(up: ShellMetrics.rowHeight + ShellMetrics.overRun, tabCount: 1) == 1)
    }

    /// The two halves of the lift meet exactly, with nothing between them.
    ///
    /// This is the assertion the page's motion rests on: `columnProgress`
    /// reaches 1 at precisely the lift where `overviewProgress` leaves 0, so a
    /// transform blended from the two of them is continuous across the join.
    /// A gap would be a stretch of the drag where the screen does not move; an
    /// overlap would be a stretch where it moves twice as fast.
    @Test func theColumnAndTheOverviewCoverTheLiftBetweenThem() {
        #expect(ShellGesture.columnProgress(up: 0, tabCount: 3) == 0)
        #expect(ShellGesture.columnProgress(up: -40, tabCount: 3) == 0)
        #expect(ShellGesture.columnProgress(up: ShellMetrics.rowHeight * 1.5, tabCount: 3) == 0.5)

        let join = ShellMetrics.rowHeight * 3
        #expect(ShellGesture.columnProgress(up: join, tabCount: 3) == 1)
        #expect(ShellGesture.overviewProgress(up: join, tabCount: 3) == 0)

        // Past the join the column has nothing left to say and stays at 1,
        // which is what leaves the rest of the travel to the overview alone.
        #expect(ShellGesture.columnProgress(up: join + ShellMetrics.overRun, tabCount: 3) == 1)

        // A workspace with no tabs has no column to be a fraction of.
        #expect(ShellGesture.columnProgress(up: 10, tabCount: 0) == 1)
    }

    /// The page does not move at all until the lift has passed the tabs.
    ///
    /// The rule the whole lift now rests on: picking a tab is a light action
    /// taken inside the workspace, so the column opens over a page that has
    /// not moved, and the page only becomes a card once the finger has gone
    /// past the last row. Asserted against the join rather than against a
    /// number, so a `rowHeight` that moves again cannot make this pass while
    /// the page steps back under an open menu.
    @Test func thePageDoesNotMoveUntilTheLiftHasPassedTheTabs() {
        let join = ShellMetrics.rowHeight * 3

        for up in stride(from: CGFloat(0), through: join, by: ShellMetrics.rowHeight / 4) {
            #expect(ShellGesture.pageRise(up: up, tabCount: 3) == 0)
            #expect(ShellGesture.columnProgress(up: up, tabCount: 3) <= 1)
        }
        #expect(ShellGesture.pageRise(up: -40, tabCount: 3) == 0)

        // And past it, one point of page for one point of finger.
        #expect(ShellGesture.pageRise(up: join + 1, tabCount: 3) == 1)
        #expect(ShellGesture.pageRise(up: join + ShellMetrics.overRun, tabCount: 3) == ShellMetrics.overRun)

        // Unclamped above, where `overviewProgress` is not: the page has
        // finished shrinking there, and it still has to follow the thumb.
        #expect(ShellGesture.pageRise(up: join + 400, tabCount: 3) == 400)
        #expect(ShellGesture.overviewProgress(up: join + 400, tabCount: 3) == 1)

        // A workspace with no tabs has no column to hold the page still.
        #expect(ShellGesture.pageRise(up: 10, tabCount: 0) == 10)
    }

    /// The page starts moving at exactly the point the overview starts
    /// arriving, for any number of tabs.
    ///
    /// Two facts about the same instant, so they are asserted together: a
    /// `pageRise` that began before `overviewProgress` would be a page that
    /// steps back to pick a tab, and one that began after would be a stretch
    /// of the drag where the overview is arriving behind a page that has not
    /// moved to reveal it.
    @Test func thePageBeginsMovingWhereTheOverviewBegins() {
        for tabs in 0...9 {
            let join = ShellGesture.columnFull(tabCount: tabs)
            #expect(ShellGesture.pageRise(up: join, tabCount: tabs) == 0)
            #expect(ShellGesture.overviewProgress(up: join, tabCount: tabs) == 0)
            #expect(ShellGesture.pageRise(up: join + 8, tabCount: tabs) == 8)
            #expect(ShellGesture.overviewProgress(up: join + 8, tabCount: tabs) > 0)
        }
    }

    /// Precedence sorts WORKSPACES, and one stale tab is not a stale
    /// workspace — only a workspace nothing has been heard from is.
    @Test func precedenceRanksWorkspacesNotTabs() {
        let working = ShellWorkspace(
            id: "w", name: "w", tabs: [Self.tab("t", .working), Self.tab("s", .stale)])
        let allStale = ShellWorkspace(
            id: "s", name: "s", tabs: [Self.tab("s0", .stale), Self.tab("s1", .stale)])
        let diff = ShellWorkspace(
            id: "d", name: "d", tabs: [Self.tab("d0", .stale), Self.tab("d1", .unreadDiff)])
        let blocked = ShellWorkspace(
            id: "b", name: "b", tabs: [Self.tab("b0", .unreadDiff), Self.tab("b1", .needsYou)])
        #expect(working.precedence == .working)
        #expect(allStale.precedence == .allStale)
        #expect(diff.precedence == .unreadDiff)
        #expect(blocked.precedence == .needsYou, "needs-you outranks a diff in the same workspace")
        #expect(ShellWorkspace(id: "e", name: "e", tabs: []).precedence == .working)
    }

    /// The overview is the fleet with the loud ones lifted, not a new
    /// arrangement: equal precedence keeps fleet order.
    @Test func theOverviewSortsByPrecedenceAndIsStable() {
        let fleet = ShellFleet(workspaces: [
            ShellWorkspace(id: "0", name: "working-a", tabs: [Self.tab("x", .working)]),
            ShellWorkspace(id: "1", name: "needs", tabs: [Self.tab("x", .needsYou)]),
            ShellWorkspace(id: "2", name: "working-b", tabs: [Self.tab("x", .working)]),
            ShellWorkspace(id: "3", name: "stale", tabs: [Self.tab("x", .stale)]),
            ShellWorkspace(id: "4", name: "diff", tabs: [Self.tab("x", .unreadDiff)]),
            ShellWorkspace(id: "5", name: "needs-later", tabs: [Self.tab("x", .needsYou)]),
        ])
        #expect(fleet.overviewOrder() == [1, 5, 4, 3, 0, 2])
    }

    /// Search is a substring over names, case- and diacritic-blind, and keeps
    /// the precedence order it filters.
    @Test func searchFiltersByNameAndKeepsTheOrder() {
        let fleet = ShellFleet(workspaces: [
            ShellWorkspace(id: "0", name: "feat/retries", tabs: [Self.tab("x", .working)]),
            ShellWorkspace(id: "1", name: "fix/RETRY-storm", tabs: [Self.tab("x", .needsYou)]),
            ShellWorkspace(id: "2", name: "chore/deps", tabs: [Self.tab("x", .working)]),
        ])
        #expect(fleet.overviewOrder(matching: "retr") == [1, 0])
        #expect(fleet.overviewOrder(matching: "DEPS") == [2])
        #expect(fleet.overviewOrder(matching: "nothing") == [])
        #expect(
            fleet.overviewOrder(matching: "   ") == fleet.overviewOrder(),
            "a stray space must not empty the grid")
    }

    // MARK: - Release

    /// The bar's four answers, at the thresholds themselves.
    @Test func theBarCommitsAtSeventyPointsAndNotAtSixtyNine() {
        let fleet = Self.crossing()
        let middle = ShellPosition(workspace: 1, tab: 2)
        #expect(fleet.barRelease(axis: .horizontal, dx: -69, up: 0, at: middle) == .springBack)
        #expect(
            fleet.barRelease(axis: .horizontal, dx: -70, up: 0, at: middle)
                == .commit(
                    ShellStep(
                        position: ShellPosition(workspace: 2, tab: 0), crossesWorkspace: true)))
        #expect(
            fleet.barRelease(axis: .horizontal, dx: 70, up: 0, at: middle)
                == .commit(
                    ShellStep(
                        position: ShellPosition(workspace: 0, tab: 0), crossesWorkspace: true)))
        // Far enough, but there is nothing there.
        #expect(
            fleet.barRelease(
                axis: .horizontal, dx: 200, up: 0, at: ShellPosition(workspace: 0, tab: 0))
                == .springBack)
    }

    @Test func theBarsVerticalArmAbandonsLandsOrOpensTheOverview() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        #expect(fleet.barRelease(axis: .vertical, dx: 0, up: 10, at: at) == .abandon)
        // The menu reads top to bottom, so the row the first points of lift
        // put under the thumb — the one nearest the bar — is the LAST tab,
        // and further lift walks upward toward tab 0.
        #expect(
            fleet.barRelease(axis: .vertical, dx: 0, up: ShellMetrics.openMin, at: at)
                == .land(tab: 2))
        #expect(
            fleet.barRelease(axis: .vertical, dx: 0, up: ShellMetrics.rowHeight + 2, at: at)
                == .land(tab: 1))
        #expect(
            fleet.barRelease(axis: .vertical, dx: 0, up: ShellMetrics.rowHeight * 3, at: at)
                == .land(tab: 0),
            "a lift that fills the column has walked all the way up to tab 0")
        #expect(fleet.barRelease(axis: .vertical, dx: 0, up: ShellMetrics.rowHeight * 3 + ShellMetrics.overRun, at: at) == .openOverview)
    }

    // MARK: - Both axes, once the page is in your hand

    /// The line the sideways half of a lift starts at: the page has to have
    /// left the display first.
    ///
    /// Exactly `pageRise > 0`, and the same point `overviewProgress` starts
    /// from — so "the column has nothing left to reveal", "the page starts
    /// moving" and "sideways starts meaning something" are one place and
    /// cannot come apart.
    @Test func sidewaysOnlyMeansSomethingOnceThePageHasLeftTheDisplay() {
        let full = ShellGesture.columnFull(tabCount: 3)
        #expect(ShellGesture.pageIsHeld(up: 0, tabCount: 3) == false)
        #expect(ShellGesture.pageIsHeld(up: full - 1, tabCount: 3) == false)
        #expect(ShellGesture.pageIsHeld(up: full, tabCount: 3) == false)
        #expect(ShellGesture.pageIsHeld(up: full + 0.5, tabCount: 3))
    }

    /// A thumb that wanders sideways while it is choosing a column row still
    /// chooses the row.
    ///
    /// The reason `pageIsHeld` gates the sideways arm at all. A 70-point
    /// wander is a big one, but it is reachable: the travel that opens a
    /// three-tab column is 132 points up a phone held in one hand, and an arc
    /// is what a thumb draws. Reading a page turn out of it would make every
    /// tab choice a coin toss.
    @Test func aSidewaysWanderWhileChoosingARowStillChoosesTheRow() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        #expect(
            fleet.barRelease(axis: .vertical, dx: -200, up: ShellMetrics.rowHeight + 2, at: at)
                == .land(tab: 1))
        #expect(fleet.barRelease(axis: .vertical, dx: 200, up: 10, at: at) == .abandon)
    }

    /// Lifted into the overview AND far enough sideways: the page is carried
    /// into the neighbouring workspace's cell.
    ///
    /// Both axes answered off one release — the lift says you are staying up,
    /// the sideways says which card you are holding when you get there.
    @Test func aLiftedPageCanBeCarriedToTheNeighbouringWorkspace() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 2)
        let up = ShellGesture.columnFull(tabCount: 3) + ShellMetrics.overRun
        #expect(
            fleet.barRelease(axis: .vertical, dx: -ShellMetrics.pageCommit, up: up, at: at)
                == .carry(ShellStep(position: ShellPosition(workspace: 2, tab: 0), crossesWorkspace: true)))
        // The bar's own step, so it lands on the neighbour's FIRST tab and not
        // on the tab you happened to be looking at.
        #expect(
            fleet.barRelease(axis: .vertical, dx: ShellMetrics.pageCommit, up: up, at: at)
                == .carry(ShellStep(position: ShellPosition(workspace: 0, tab: 0), crossesWorkspace: true)))
        // A point short of the commit is not a carry, and the lift's own
        // answer stands.
        #expect(
            fleet.barRelease(axis: .vertical, dx: -(ShellMetrics.pageCommit - 1), up: up, at: at)
                == .openOverview)
    }

    /// At the ends of the fleet there is no neighbour to carry to, and the
    /// lift's answer stands on its own.
    @Test func aCarryOffTheEndOfTheFleetIsJustTheOverview() {
        let fleet = Self.crossing()
        let first = ShellPosition(workspace: 0, tab: 0)
        let last = ShellPosition(workspace: 2, tab: 0)
        #expect(
            fleet.barRelease(
                axis: .vertical, dx: 200,
                up: ShellGesture.columnFull(tabCount: 2) + ShellMetrics.overRun, at: first)
                == .openOverview)
        #expect(
            fleet.barRelease(
                axis: .vertical, dx: -200,
                up: ShellGesture.columnFull(tabCount: 1) + ShellMetrics.overRun, at: last)
                == .openOverview)
    }

    /// Lifted, but not far enough to stay up, and flicked sideways: the page
    /// comes back down onto the neighbour.
    ///
    /// The composition the two axes are supposed to have — the height decides
    /// whether you end up in the overview, the sideways decides which
    /// workspace — read at the height where the first answer is "no".
    @Test func aPartialLiftFlickedSidewaysIsAnOrdinaryCrossing() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 2)
        let up = ShellGesture.columnFull(tabCount: 3) + 1
        #expect(
            fleet.barRelease(axis: .vertical, dx: -200, up: up, at: at)
                == .commit(ShellStep(position: ShellPosition(workspace: 2, tab: 0), crossesWorkspace: true)))
    }

    /// No axis is a tap, and a tap is the only thing that toggles the pin.
    @Test func noAxisIsATap() {
        let fleet = Self.crossing()
        #expect(
            fleet.barRelease(axis: nil, dx: 3, up: 2, at: ShellPosition(workspace: 0, tab: 0))
                == .toggleColumn)
    }

    /// The content commits along the flat sequence, and a vertical lock on the
    /// content resolves to nothing — the pane keeps its own scroll gesture.
    @Test func theContentCommitsAlongTheFlatSequence() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)
        #expect(
            fleet.contentRelease(axis: .horizontal, dx: -70, at: at)
                == .commit(
                    ShellStep(
                        position: ShellPosition(workspace: 1, tab: 1), crossesWorkspace: false)))
        #expect(
            fleet.contentRelease(axis: .horizontal, dx: 70, at: at)
                == .commit(
                    ShellStep(
                        position: ShellPosition(workspace: 0, tab: 1), crossesWorkspace: true)))
        #expect(fleet.contentRelease(axis: .vertical, dx: -200, at: at) == .springBack)
        #expect(fleet.contentRelease(axis: nil, dx: -200, at: at) == .springBack)
    }

    /// A fleet that emptied under the finger springs back rather than trapping.
    @Test func aPositionTheFleetNoLongerHasIsSurvivable() {
        let fleet = ShellFleet(workspaces: [])
        let gone = ShellPosition(workspace: 4, tab: 9)
        #expect(fleet.step(from: gone, .next, along: .content) == nil)
        #expect(fleet.barRelease(axis: .horizontal, dx: -200, up: 0, at: gone) == .springBack)
        #expect(fleet.barRelease(axis: .vertical, dx: 0, up: 200, at: gone) == .openOverview)
        #expect(fleet.tabCount(ofWorkspace: 4) == 0)
        #expect(fleet.first == nil)
    }

    /// The rail tracks the content proportionally, which at the prototype's
    /// width is the doc's 361 to 393.
    @Test func theRailIsThePageLessTheBarsTwoInsets() {
        #expect(ShellMetrics.railWidth() == 361)
        #expect(ShellMetrics.railWidth(page: 440) == 408)
    }
}
