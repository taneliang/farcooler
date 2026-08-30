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

    /// The bar walks WORKSPACES, and always at tab 0 — in both directions.
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

    /// One row revealed per 34 points, capped at the workspace's tab count,
    /// and no selection at all below 16.
    @Test func theColumnRevealsOneRowPerThirtyFourPoints() {
        #expect(ShellGesture.columnSelection(up: 0, tabCount: 3) == nil)
        #expect(ShellGesture.columnSelection(up: 15.9, tabCount: 3) == nil)
        #expect(ShellGesture.columnSelection(up: 16, tabCount: 3) == 0)
        #expect(ShellGesture.columnSelection(up: 34, tabCount: 3) == 0)
        #expect(ShellGesture.columnSelection(up: 35, tabCount: 3) == 1)
        #expect(ShellGesture.columnSelection(up: 68, tabCount: 3) == 1)
        #expect(ShellGesture.columnSelection(up: 400, tabCount: 3) == 2, "capped at the last row")
        #expect(ShellGesture.columnSelection(up: 400, tabCount: 0) == nil)
    }

    /// Pinned open is a SEPARATE input from the lift, and outranks it. This is
    /// the separation the prototype keeps `colOpen` distinct from `dragY` for:
    /// a column that derived its height from both would close itself the
    /// instant a tap-opened column was touched again.
    @Test func aPinnedColumnIsFullHeightWhateverTheLiftSays() {
        #expect(ShellGesture.columnHeight(up: 0, tabCount: 3, pinned: true) == 102)
        #expect(ShellGesture.columnHeight(up: 0, tabCount: 3, pinned: false) == 0)
        #expect(ShellGesture.columnHeight(up: 50, tabCount: 3, pinned: false) == 50)
        #expect(
            ShellGesture.columnHeight(up: 500, tabCount: 3, pinned: false) == 102,
            "the column never grows past its rows")
        #expect(
            ShellGesture.columnHeight(up: -80, tabCount: 3, pinned: false) == 0,
            "a downward drag does not give the column a negative height")
    }

    // MARK: - The overview

    /// The overview starts where the column runs out, so it is always the same
    /// gesture — keep going — whether the workspace has one tab or nine.
    @Test func theOverviewBeginsWhereTheColumnRunsOut() {
        #expect(ShellGesture.overviewProgress(up: 102, tabCount: 3) == 0)
        #expect(ShellGesture.overviewProgress(up: 140, tabCount: 3) == 0.5)
        #expect(ShellGesture.overviewProgress(up: 178, tabCount: 3) == 1)
        #expect(ShellGesture.overviewProgress(up: 400, tabCount: 3) == 1)
        #expect(ShellGesture.overviewProgress(up: 0, tabCount: 3) == 0)
        // A one-tab workspace reaches it 68 points sooner, and that is the
        // point: the distance is measured from the end of the column.
        #expect(ShellGesture.overviewProgress(up: 110, tabCount: 1) == 1)
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
        #expect(fleet.barRelease(axis: .vertical, dx: 0, up: 16, at: at) == .land(tab: 0))
        #expect(fleet.barRelease(axis: .vertical, dx: 0, up: 90, at: at) == .land(tab: 2))
        #expect(fleet.barRelease(axis: .vertical, dx: 0, up: 177, at: at) == .land(tab: 2))
        #expect(fleet.barRelease(axis: .vertical, dx: 0, up: 178, at: at) == .openOverview)
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
