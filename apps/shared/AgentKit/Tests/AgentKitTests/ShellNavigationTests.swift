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

    /// The shorthand these tests are written in.
    ///
    /// **A TEST vocabulary, and deliberately no longer the app's.** `ShellMark`
    /// was an enum of exactly these names in the app itself, and it is retired
    /// — `ShellTab.mark` says why. What the navigation and precedence tests
    /// need is still a one-word way to say "a workspace with a blocked tab in
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

    // MARK: - The first six points

    /// Nothing under 6 points has an axis at all, which is what stops a tap
    /// from being a tiny drag.
    ///
    /// `lean` with nothing to lean from is this same function, so a gesture
    /// on the BAR begins exactly where a gesture on the content begins. That
    /// is the whole of why unlocking the axis changed no straight drag: it
    /// changed no FIRST answer.
    @Test func theAxisIsUndecidedUnderSixPoints() {
        #expect(ShellGesture.axis(dx: 0, up: 0) == nil)
        #expect(ShellGesture.axis(dx: 5.9, up: 0) == nil)
        #expect(ShellGesture.axis(dx: 0, up: 5.9) == nil)
        #expect(ShellGesture.axis(dx: -5.9, up: -5.9) == nil)
        #expect(ShellGesture.axis(dx: 6.1, up: 0) == .horizontal)
        #expect(ShellGesture.axis(dx: 0, up: 6.1) == .vertical)

        #expect(ShellGesture.lean(dx: 5.9, up: 0, from: nil) == nil)
        #expect(ShellGesture.lean(dx: 6.1, up: 0, from: nil) == .horizontal)
        #expect(ShellGesture.lean(dx: 0, up: 6.1, from: nil) == .vertical)
        // The tie, and the answer to "what wins at the diagonal" for a
        // gesture with no history: vertical, because the comparison is `>`.
        // It matters only here — after this there IS a history, and the
        // diagonal is handed to it.
        #expect(ShellGesture.lean(dx: 40, up: 40, from: nil) == .vertical)
    }

    /// **A drag down the screen is VERTICAL, however far it wanders
    /// sideways.**
    ///
    /// This asserted the opposite, from the prototype: `abs(dx) > dy` with
    /// `dy` up-positive, so a downward drag lost to any horizontal component
    /// at all and was called horizontal. It was harmless on the bar for
    /// exactly the reason the old comment gave — a downward flick's `dx` is
    /// nowhere near the commit threshold — and it was the owner's bug on the
    /// CONTENT, where a thumb travelling six hundred points down a terminal
    /// arcs eighty across on the way and eighty is past the seventy that
    /// commits.
    ///
    /// Both halves are pinned here: the plain downward flick that used to be
    /// the point of this test, and the drifting one that was the bug.
    @Test func aDragDownTheScreenIsVerticalHoweverFarItDrifts() {
        #expect(ShellGesture.axis(dx: 0, up: -40) == .vertical)
        #expect(ShellGesture.axis(dx: 79, up: -511) == .vertical)
        #expect(ShellGesture.axis(dx: -79, up: -511) == .vertical)
        // And a drag that really is sideways still is, whichever way it leans.
        #expect(ShellGesture.axis(dx: 120, up: -40) == .horizontal)
        #expect(ShellGesture.axis(dx: 120, up: 40) == .horizontal)
    }

    /// The other half of the old test, which was the part worth keeping: a
    /// downward flick on the bar costs nothing. It reaches that as an
    /// `.abandon` now rather than as a `.springBack`, and the two are the same
    /// nothing — `apply` runs `flatten()` for both.
    @Test func aDownwardFlickOnTheBarCostsNothing() {
        let fleet = Self.crossing()
        #expect(
            fleet.barRelease(
                axis: .vertical, dx: 0, up: 0, at: ShellPosition(workspace: 1, tab: 0))
                == .abandon)
    }

    // MARK: - Redirection

    /// One frame of a bar drag, folded over a whole path the way the frame
    /// loop does.
    ///
    /// `ShellDrag.barGesture` is the other caller of exactly this, and it
    /// adds only pixels: `frame.axis` picks the arm, `frame.lift` and
    /// `frame.sideways` are what it writes, `frame.claimed` runs the eased
    /// put-back. So a path driven through here is the shipped redirection
    /// and not a replica of it.
    private func drive(_ path: [(dx: CGFloat, up: CGFloat)], tabCount: Int)
        -> (drag: ShellBarDrag, frame: ShellBarDrag.Frame, flips: Int)
    {
        var drag = ShellBarDrag()
        var frame = ShellBarDrag.Frame(axis: nil, lift: 0, sideways: 0, claimed: nil)
        var flips = 0
        for point in path {
            frame = drag.moved(dx: point.dx, up: point.up, tabCount: tabCount)
            if frame.claimed != nil { flips += 1 }
        }
        return (drag, frame, flips)
    }

    /// A straight run of samples between two corners, the way a finger
    /// travelling at a constant speed arrives.
    private func leg(
        from: (dx: CGFloat, up: CGFloat), to: (dx: CGFloat, up: CGFloat), steps: Int = 24
    ) -> [(dx: CGFloat, up: CGFloat)] {
        (1...steps).map { i in
            let t = CGFloat(i) / CGFloat(steps)
            return (dx: from.dx + (to.dx - from.dx) * t, up: from.up + (to.up - from.up) * t)
        }
    }

    /// **A drag that goes sideways and then turns upward opens the column.**
    /// The owner's ask, in one direction: *"the user can start swiping
    /// horizontally, then decide they want to swipe vertically instead."*
    ///
    /// Nothing in this shell asserted it before, and nothing could: the axis
    /// was decided on the first six points and never revisited, so a gesture
    /// that began sideways was a page turn until the finger left the glass
    /// however far up the phone it went. WWDC 2018 803 on the cost of that —
    /// *"you'd have to think what you want to do… then perform the gesture"*
    /// — against what it buys: *"the thought and gesture happen in parallel.
    /// And you sort of think it with the gesture, and it turns out this is
    /// way faster than thinking before doing."*
    ///
    /// Ninety points sideways is well past the seventy that commits, so under
    /// the lock this gesture WAS a workspace crossing and the counter-example
    /// below says so in the same breath. The turn is then straight up, and
    /// the axis changes hands at 126 points of lift — 1.4 × 90, which is
    /// `ShellMetrics.redirect` and nothing else.
    @Test func aDragThatTurnsUpwardOutOfASidewaysSwipeReachesTheColumn() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        let path = leg(from: (0, 0), to: (-90, 0)) + leg(from: (-90, 0), to: (-90, 200))
        let run = drive(path, tabCount: 3)

        #expect(run.frame.axis == .vertical, "the gesture never changed its mind")
        #expect(run.flips == 1, "it changed its mind once, not once per frame")
        // The sideways travel it spent on the page turn it abandoned, which
        // is what stops the carried card jumping ninety points across the
        // display the moment the vertical claims the gesture.
        #expect(abs(run.drag.spentSideways + 90) < 0.01)
        #expect(run.frame.sideways == 0, "and so the sideways channel starts from nothing")
        // The gesture changes hands at 126 points of lift, a hair inside a
        // three-tab column's 132, so there is next to nothing of the page's
        // own travel to charge — the frame it lands on overshoots by a point
        // and a third, and a point and a third is what it costs.
        #expect(run.drag.spentLift < 2)
        #expect(
            abs(run.frame.lift - (200 - run.drag.spentLift)) < 0.01,
            "the column's own stretch is adopted whole; only the page's is charged")

        // And what letting go of it does. The finger is 200 points up, which
        // is past the last row of a three-tab column, so there is no row to
        // land on and the run past it is what the release reads.
        #expect(
            fleet.barRelease(
                axis: run.frame.axis, dx: run.frame.sideways, up: run.frame.lift, at: at,
                row: ShellGesture.columnRow(above: 200, tabCount: 3))
                == .abandon,
            "past the last row there is no row, and 200 is short of the overview's run")
        // And carried on to where the overview's own run ends, it leaves.
        let further = drive(
            leg(from: (0, 0), to: (-90, 0)) + leg(from: (-90, 0), to: (-90, 340)), tabCount: 3)
        #expect(
            fleet.barRelease(
                axis: further.frame.axis, dx: further.frame.sideways, up: further.frame.lift,
                at: at, row: nil)
                == .openOverview)
        // The same finger, one row down the column, chooses the row — which
        // is the outcome that was unreachable, not merely the axis.
        let shorter = leg(from: (0, 0), to: (-90, 0)) + leg(from: (-90, 0), to: (-90, 145))
        let short = drive(shorter, tabCount: 3)
        #expect(short.frame.axis == .vertical)
        #expect(
            fleet.barRelease(
                axis: short.frame.axis, dx: short.frame.sideways, up: short.frame.lift, at: at,
                row: ShellGesture.columnRow(above: 100, tabCount: 3))
                == .land(tab: 0))

        // The counter-example, and it is the same ninety points: a gesture
        // that does NOT turn is the workspace crossing it always was.
        let straight = drive(leg(from: (0, 0), to: (-90, 0)), tabCount: 3)
        #expect(straight.frame.axis == .horizontal)
        #expect(straight.flips == 0)
        #expect(
            fleet.barRelease(
                axis: straight.frame.axis, dx: straight.frame.sideways, up: 0, at: at)
                == .commit(
                    ShellStep(
                        position: ShellPosition(workspace: 2, tab: 0), crossesWorkspace: true)))
    }

    /// **And the mirror: a lift that turns sideways crosses the workspace.**
    /// *"…or vice versa."*
    ///
    /// The half that looks like it already worked and did not. A lifted page
    /// COULD go sideways — `.carry` — but only past the last row, where the
    /// page is off the display and the two axes have stopped competing. Below
    /// it, inside the column, sideways meant nothing at all however decisive
    /// it was: `aSidewaysWanderWhileChoosingARowStillChoosesTheRow` is the
    /// test that said so, and it said it with two hundred points.
    ///
    /// A hundred points of lift into a three-tab column is a menu open with a
    /// row lit under the thumb. From there it takes 140 points sideways to
    /// take the gesture back — 1.4 × 100 — and that is the price being right
    /// rather than incidental: leaving a menu you have opened and are reading
    /// should look like a decision, and the ratio makes it cost more the
    /// further into the menu you are.
    @Test func aLiftThatTurnsSidewaysOutOfAnOpenColumnCrossesTheWorkspace() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 2)  // three tabs
        let path = leg(from: (0, 0), to: (0, 100)) + leg(from: (0, 100), to: (-220, 100))
        let run = drive(path, tabCount: 3)

        #expect(run.frame.axis == .horizontal, "the lift never gave the gesture up")
        #expect(run.flips == 1)
        #expect(
            run.drag.spentLift == 0,
            "a charge on the lift belongs to whoever is holding the vertical")
        // Charged for the 140-odd points it spent inside the column, so the
        // page turn begins from nothing rather than jumping most of the way
        // to the neighbour in the frame the gesture changed its mind.
        #expect(run.drag.spentSideways < -139 && run.drag.spentSideways > -150)
        #expect(
            run.frame.sideways > -81 && run.frame.sideways < -70,
            "and what is left still earns its own seventy points")
        #expect(
            fleet.barRelease(axis: run.frame.axis, dx: run.frame.sideways, up: 0, at: at)
                == .commit(
                    ShellStep(
                        position: ShellPosition(workspace: 2, tab: 0), crossesWorkspace: true)))

        // The counter-example: the same lift, turned sideways by a hundred
        // points rather than two hundred and twenty. That is a long way for a
        // thumb to wander and it is still short of taking the gesture, so the
        // column keeps it and the row under the finger is what you get.
        let wandered = drive(
            leg(from: (0, 0), to: (0, 100)) + leg(from: (0, 100), to: (-100, 100)),
            tabCount: 3)
        #expect(wandered.frame.axis == .vertical)
        #expect(wandered.flips == 0)
        #expect(
            fleet.barRelease(
                axis: wandered.frame.axis, dx: wandered.frame.sideways, up: wandered.frame.lift,
                at: at, row: ShellGesture.columnRow(above: 100, tabCount: 3))
                == .land(tab: 0))
    }

    /// **A straight drag means exactly what it meant before any of this.**
    ///
    /// The negative control for the whole change, and it is provable rather
    /// than sampled: along a straight line `|dx| : |dy|` is constant, so an
    /// incumbent axis is by construction already the larger of the two and
    /// can never be beaten by `redirect` times itself. Every angle, both
    /// signs, and the answer is `axis` on the endpoint every time.
    ///
    /// It is also why the UI suite's eight bar tests did not have to change a
    /// number: `XCUIElement.press(forDuration:thenDragTo:)` interpolates a
    /// straight line, so not one gesture anybody can synthesize redirects.
    @Test func aStraightDragMeansExactlyWhatItAlwaysDid() {
        for degrees in stride(from: 0, through: 355, by: 5) {
            let radians = CGFloat(degrees) * .pi / 180
            let end: (dx: CGFloat, up: CGFloat) = (300 * cos(radians), 300 * sin(radians))
            let run = drive(leg(from: (0, 0), to: end, steps: 60), tabCount: 3)
            #expect(
                run.frame.axis == ShellGesture.axis(dx: end.dx, up: end.up),
                "a straight drag at \(degrees)° changed its meaning")
            #expect(run.flips == 0, "a straight drag at \(degrees)° changed its mind")
            #expect(run.drag.spentSideways == 0)
            #expect(run.drag.spentLift == 0)
        }
    }

    /// **The diagonal keeps whatever the gesture already is, and the band it
    /// keeps it through is 19° wide.**
    ///
    /// This is what a hysteresis IS, and the answer to "what happens at the
    /// diagonal": nothing happens at the diagonal. Without one the comparison
    /// is `abs(dx) > abs(dy)` in both directions, so a finger travelling
    /// within a point of 45° resolves by rounding and re-resolves the other
    /// way on the next sample — and each of those is `ShellMotion.menu`'s own
    /// 0.28-second spring being told to open and then to shut, over a surface
    /// that never finishes arriving.
    ///
    /// The two edges are stated as numbers so the band can be argued with.
    /// From horizontal it takes 54.5° off horizontal to be let go of; from
    /// vertical it takes 35.5°. `atan(1.4)` and its complement, which is what
    /// makes `ShellMetrics.redirect` an angle rather than a distance.
    @Test func theDiagonalKeepsWhateverTheGestureAlreadyIs() {
        // Dead on 45°, from either side, is whatever it already was.
        #expect(ShellGesture.lean(dx: 100, up: 100, from: .horizontal) == .horizontal)
        #expect(ShellGesture.lean(dx: 100, up: 100, from: .vertical) == .vertical)
        #expect(ShellGesture.lean(dx: -100, up: -100, from: .horizontal) == .horizontal)

        // The band's edges. `tan 54.5° = 1.4`, so a hair past it the
        // horizontal lets go and a hair short of it does not.
        #expect(ShellGesture.lean(dx: 100, up: 141, from: .horizontal) == .vertical)
        #expect(ShellGesture.lean(dx: 100, up: 139, from: .horizontal) == .horizontal)
        #expect(ShellGesture.lean(dx: 141, up: 100, from: .vertical) == .horizontal)
        #expect(ShellGesture.lean(dx: 139, up: 100, from: .vertical) == .vertical)

        // And the strobe it exists to prevent, driven as a path: a finger
        // crawling up the exact diagonal and landing a point either side of
        // it, which is what integer input on a 45° path looks like. The
        // memoryless rule is written out below as the counter-example it is,
        // the way `theChosenRowFollowsTheFinger` writes out the delta mapping
        // it replaced — so this test says what changed and not merely what
        // is.
        var jittered: [(dx: CGFloat, up: CGFloat)] = []
        for i in 1...120 {
            let t = CGFloat(i)
            jittered.append((dx: t + (i % 2 == 0 ? 1 : -1), up: t))
        }
        var memoryless = 0
        var last: ShellAxis?
        for point in jittered {
            let answer = ShellGesture.axis(dx: point.dx, up: point.up)
            if let answer, answer != last { memoryless += 1 }
            last = answer ?? last
        }
        #expect(
            memoryless > 100,
            "the rule with no memory answers a different axis on nearly every frame")

        let run = drive(jittered, tabCount: 3)
        #expect(run.flips == 0, "the shell changed its mind mid-diagonal")
        // Whichever it first answered, and it keeps it. The first sample past
        // the axis lock lands a point to the horizontal side, so that is the
        // answer for all 120 frames — which is the property, rather than the
        // side it happened to land on.
        #expect(run.frame.axis == .horizontal)
    }

    /// A gesture that has moved is never a tap again, however far back toward
    /// the origin it comes.
    ///
    /// The trap in re-asking a question that used to be asked once: the tap
    /// is the ABSENCE of an axis, so a rule that could answer nil a second
    /// time would make a drag out and back resolve to `.toggleColumn` —
    /// opening or shutting the column under a finger that had plainly asked
    /// for neither.
    @Test func aGestureThatHasMovedIsNeverATapAgain() {
        #expect(ShellGesture.lean(dx: 0, up: 0, from: .horizontal) == .horizontal)
        #expect(ShellGesture.lean(dx: 0, up: 0, from: .vertical) == .vertical)
        let run = drive(
            leg(from: (0, 0), to: (60, 0)) + leg(from: (60, 0), to: (0, 0)), tabCount: 3)
        #expect(run.frame.axis == .horizontal)
    }

    /// A charge handed back is a charge dropped.
    ///
    /// `spentLift` describes what the VERTICAL was given, so it means nothing
    /// while the horizontal holds the gesture — and left standing it is a
    /// phantom: `ShellRootView.fingerLift` adds it to a lift the horizontal
    /// arm has just zeroed, which puts a thumb 80 points above a column that
    /// a tap is holding open and shuts a menu nobody asked to shut.
    @Test func aChargeOnTheLiftIsDroppedWhenTheHorizontalTakesTheGestureBack() {
        // The one shape that can reach it, and it is narrow on purpose. A
        // charge is only non-zero when the vertical claims a gesture already
        // past the last row — and the charge itself puts the lift back ON the
        // last row, which is not yet `pageIsHeld`, so the page is not in your
        // hand and the lean is still being asked. One point more of RISE and
        // it would be held and this could never happen; so the finger turns
        // sideways instead, and takes the gesture back.
        let path =
            leg(from: (0, 0), to: (-143, 0)) + leg(from: (-143, 0), to: (-143, 201))
            + leg(from: (-143, 201), to: (-400, 201))
        let run = drive(path, tabCount: 3)
        #expect(!run.drag.holdingPage, "the page never left the display, so nothing is held")
        #expect(run.frame.axis == .horizontal)
        #expect(run.flips == 2, "it changed its mind twice")
        #expect(run.drag.spentLift == 0)
        // And the sideways charge is not dropped but RE-TAKEN, because that
        // one is a fact about the gesture rather than about either axis: it
        // is however far sideways this gesture had gone the last time it
        // changed its mind, which is 1.4 times a 201-point lift.
        #expect(run.drag.spentSideways < -281 && run.drag.spentSideways > -290)
        #expect(
            run.frame.sideways > -119 && run.frame.sideways < -110,
            "so the page turn it hands back to still earns its own seventy points")
    }

    /// **Once the page is in your hand the lean stops being asked**, and
    /// nothing sideways can put the page back on the display.
    ///
    /// The bound on the whole change, and the one place the old lock was
    /// protecting something real. Past the last row the two axes have stopped
    /// competing and started composing — the lift says whether you stay up,
    /// sideways says which cell you land in, and `barRelease`'s `.carry` arm
    /// has answered both off one release since before any of this. Handing
    /// that gesture to `.horizontal` because the thumb went far enough
    /// sideways would drop the page onto the display while your thumb was
    /// still up in the air holding it.
    ///
    /// Latched rather than recomputed: the second half drives the card back
    /// DOWN out of the overview's run, which is a thumb lowering rather than
    /// a thumb letting go.
    @Test func theLiftClaimsBothAxesOnceThePageIsInYourHand() {
        #expect(ShellGesture.lean(dx: 900, up: 10, from: .vertical, holdingPage: true) == .vertical)
        #expect(ShellGesture.lean(dx: 900, up: 10, from: nil, holdingPage: true) == .vertical)

        // A three-tab column runs out at 132 and the overview's run is 76
        // past that, so 260 is a page well clear of the display.
        // 500 across against 260 up is past `redirect` twice over, so the
        // latch is the only thing keeping this vertical.
        let carried = drive(
            leg(from: (0, 0), to: (0, 260)) + leg(from: (0, 260), to: (-500, 260)), tabCount: 3)
        #expect(carried.frame.axis == .vertical, "a held page was handed to the other axis")
        #expect(carried.flips == 0)
        #expect(carried.drag.holdingPage)
        #expect(abs(carried.frame.sideways + 500) < 0.01, "and it follows the finger sideways")

        // Brought back down to the middle of the column, and still in your
        // hand. Recomputed rather than latched, this is where the card would
        // fall out of it.
        let lowered = drive(
            leg(from: (0, 0), to: (0, 260)) + leg(from: (0, 260), to: (-300, 100)), tabCount: 3)
        #expect(lowered.frame.axis == .vertical)
        #expect(lowered.flips == 0)
    }

    /// **A handover charges the claimed axis exactly what the page would have
    /// jumped, and not a point more.**
    ///
    /// The two halves of a handover are two different rules, and the reason
    /// is what each channel IS. The track is a POSITION — the page is AT
    /// `trackX` — so it re-bases whole: `carriedX`'s rule at a pane's edge,
    /// *"a page turn that starts from nothing rather than jumping to wherever
    /// the finger had got to"*, restated for an axis. The column is not a
    /// position: it is shut, then whole, and the row it lights is read off
    /// the finger's absolute place on the glass, so there is no in-between to
    /// jump through and nothing is bought by making a finger already 200
    /// points above the bar travel 16 more before the menu appears.
    ///
    /// What IS a position on the vertical is the page's own rise past the
    /// last row, and that is `ShellGesture.pageRise` exactly — which is why
    /// the charge is that function and not a fresh subtraction.
    @Test func aHandoverChargesTheLiftOnlyForWhatWouldHaveMovedThePage() {
        // Inside the column: nothing to charge, and the menu opens under the
        // finger the moment the gesture turns.
        let inside = drive(
            leg(from: (0, 0), to: (-60, 0)) + leg(from: (-60, 0), to: (-60, 120)), tabCount: 3)
        #expect(inside.frame.axis == .vertical)
        #expect(inside.drag.spentLift == 0)
        #expect(abs(inside.frame.lift - 120) < 0.01)

        // Past it: a gesture that went 143 sideways only lets go at 200 up,
        // which is 68 past a three-tab column's last row. Charge those 68 and
        // the page is flat on the display at the instant of the handover, so
        // it travels its own full run from there instead of teleporting two
        // thirds of the way into the overview.
        let past = drive(
            leg(from: (0, 0), to: (-143, 0)) + leg(from: (-143, 0), to: (-143, 260)),
            tabCount: 3)
        #expect(past.frame.axis == .vertical)
        #expect(past.drag.spentLift > 60 && past.drag.spentLift < 76)
        #expect(
            !ShellGesture.pageIsHeld(
                up: past.drag.spentLift == 0
                    ? 1 : ShellGesture.columnFull(tabCount: 3), tabCount: 3),
            "the page is on the display at the handover, by construction")

        // **The charge never loses the finger's place**, and the whole of
        // `ShellRootView.menuShouldShow` rests on it: the lift a gesture has
        // been GIVEN and the height the finger is actually AT differ by
        // exactly `spentLift`, so adding it back recovers the thumb.
        //
        // Asking the charged lift where the finger was is a menu that lies
        // for one frame, and it was found by watching the frames rather than
        // by reading the code. The charge puts a handover past the last row
        // at exactly `columnFull` — inside the column by a hair — so the
        // menu sprang open with the thumb 80 points above every row in it,
        // and shut again on the next frame. `ShellMotion.menu` is a
        // 0.28-second spring; what that draws is a blink.
        for up in stride(from: CGFloat(0), through: 400, by: 17) {
            var drag = ShellBarDrag()
            var frame = ShellBarDrag.Frame(axis: nil, lift: 0, sideways: 0, claimed: nil)
            for point in leg(from: (0, 0), to: (-143, 0)) + leg(from: (-143, 0), to: (-143, up)) {
                frame = drag.moved(dx: point.dx, up: point.up, tabCount: 3)
            }
            #expect(
                abs(frame.lift + drag.spentLift - max(0, up)) < 0.01,
                "the finger's place went missing at \(up) points of lift")
        }

        // A one-tab workspace is where an uncharged handover was worst: its
        // column runs out after 44 points, so the same turn would have put
        // the page most of the way into the overview in one frame.
        let single = drive(
            leg(from: (0, 0), to: (-60, 0)) + leg(from: (-60, 0), to: (-60, 120)), tabCount: 1)
        #expect(single.frame.axis == .vertical)
        #expect(abs(single.drag.spentLift - 40) < 2, "84 at the turn, less a 44-point column")
        #expect(
            ShellGesture.overviewProgress(up: 84 - single.drag.spentLift, tabCount: 1) == 0,
            "so the page starts the overview's run at nothing, not halfway through it")
    }

    // MARK: - The page's own join, and where the drag started

    /// **The page's own join moves with WHERE THE DRAG STARTED, not with how
    /// far it has travelled — the owner's report, pinned in screen terms.**
    ///
    /// A touch forty points short of the bar's own top edge — a few points
    /// off the very bottom of a 44-point bar row — has to travel forty
    /// points FURTHER than one that starts at the top edge before the page
    /// may rise, because "past the last row" is a place on the glass and the
    /// touch-down point was not that place. Before `startAbove` existed
    /// `ShellBarDrag.moved` fed `ShellGesture.pageRise` the raw travel since
    /// touch-down — `up` alone — which crossed a three-tab column's 132-point
    /// join at 132 points of travel whichever height the finger touched down
    /// at. Reverting `above` to `up + 0` below (ignoring `startAbove`)
    /// reproduces exactly that: this test goes red at the FIRST expectation,
    /// "the finger is forty points short of the real top edge", with
    /// `pageRise` reporting 1 rather than 0.
    @Test func thePageJoinMovesWithWhereTheDragStarted() {
        let tabs = 3
        let join = ShellGesture.columnFull(tabCount: tabs)  // 132
        let startedLow: CGFloat = -40

        var lowStart = ShellBarDrag(startAbove: startedLow)
        // Travel alone reaches the join at 132 — the OLD reading, and the
        // one the owner reported: the page starting to move with the
        // fingertip still over the topmost row.
        let atOldJoin = lowStart.moved(dx: 0, up: join, tabCount: tabs)
        #expect(
            ShellGesture.pageRise(up: atOldJoin.above, tabCount: tabs) == 0,
            "the finger is forty points short of the real top edge; the page must not have risen")

        // The finger's OWN place reaches the join forty points later — at
        // 172, `join - startedLow` — which is where the page may finally
        // move, and not a point before.
        let atRealJoin = lowStart.moved(dx: 0, up: join - startedLow, tabCount: tabs)
        #expect(ShellGesture.pageRise(up: atRealJoin.above, tabCount: tabs) == 0)
        let pastRealJoin = lowStart.moved(dx: 0, up: join - startedLow + 1, tabCount: tabs)
        #expect(ShellGesture.pageRise(up: pastRealJoin.above, tabCount: tabs) == 1)

        // And it is the SAME boundary in screen terms wherever the drag
        // began. A touch at the bar's own top edge reaches it at 132 points
        // of travel exactly — `ShellGesture.columnFull` itself — which is
        // forty points sooner in TRAVEL than the low touch above, but the
        // same POINT on the glass: both fingers are, at their own join,
        // exactly `columnFull` above the bar's own top edge.
        var topStart = ShellBarDrag(startAbove: 0)
        let topJoin = topStart.moved(dx: 0, up: join, tabCount: tabs)
        #expect(ShellGesture.pageRise(up: topJoin.above, tabCount: tabs) == 0)
        let topPast = topStart.moved(dx: 0, up: join + 1, tabCount: tabs)
        #expect(ShellGesture.pageRise(up: topPast.above, tabCount: tabs) == 1)
    }

    /// **The join is unaffected by a redirect that has already charged the
    /// lift**, which is the case `startAbove`'s header argues by name: once
    /// a handover has re-based the vertical channel, `lift` already IS the
    /// place that channel was granted, and `above` reads it rather than
    /// adding `startAbove` on top of it a second time.
    ///
    /// This is the negative control for the fix: every existing redirection
    /// test in this file drives a `ShellBarDrag()` with the default
    /// `startAbove` of zero, so this pins the OTHER half — a real anchor,
    /// carried through a real handover — so a future change cannot "fix" the
    /// straight-drag case by double-charging the redirected one.
    @Test func aChargedHandoverIsNotChargedTwiceByTheStartingAnchor() {
        let tabs = 3
        // A touch forty points low in the bar, that then wanders sideways
        // 143 points before turning upward — the shape
        // `aHandoverChargesTheLiftOnlyForWhatWouldHaveMovedThePage` drives
        // for its "past" case, with an anchor added.
        var drag = ShellBarDrag(startAbove: -40)
        var frame = ShellBarDrag.Frame(axis: nil, lift: 0, sideways: 0, claimed: nil)
        for point in leg(from: (0, 0), to: (-143, 0)) + leg(from: (-143, 0), to: (-143, 260)) {
            frame = drag.moved(dx: point.dx, up: point.up, tabCount: tabs)
        }
        #expect(frame.axis == .vertical)
        #expect(drag.spentLift > 0, "the handover happened after the column's own run")
        // `above` is exactly `lift` at and after the charge — not `lift`
        // shifted by another forty points — which is what this asserts.
        #expect(frame.above == frame.lift)
    }

    /// **A release reads the same real place the drag was drawing**, which
    /// is what keeps a release from doing something the screen never showed
    /// as imminent.
    ///
    /// The same two hundred and twenty points of travel, from a touch forty
    /// points low in the bar: the OLD basis — `above` omitted, so `up` reads
    /// for itself, which is every caller before this change — has already
    /// travelled past a three-tab column's overview run and leaves for the
    /// grid. The finger's actual place is forty points short of that, still
    /// short of even the column's own row margin, and abandons instead —
    /// which is the release-time half of the owner's report: a drag that
    /// never visibly reached the overview must not open it.
    @Test func aReleaseReadsTheFingersRealPlaceRatherThanTheRawTravel() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        let travelled: CGFloat = 220
        let realPlace: CGFloat = 180  // travelled - 40, the low start

        #expect(
            fleet.barRelease(
                axis: .vertical, dx: 0, up: travelled, at: at,
                row: ShellGesture.columnRow(above: travelled, tabCount: 3))
                == .openOverview,
            "the default reading is `up` itself, which is every caller before `above` existed")

        #expect(
            fleet.barRelease(
                axis: .vertical, dx: 0, up: travelled, at: at,
                row: ShellGesture.columnRow(above: realPlace, tabCount: 3), above: realPlace)
                == .abandon,
            "the finger's real place is short of the column's own margin — nothing was picked, and nothing moved")
    }

    // MARK: - Tapping a column row

    /// **A tap on an open column row chooses that row.**
    ///
    /// The column had no target of its own, so a tap anywhere on that surface
    /// reached `barRelease` with no axis and resolved to `.toggleColumn` —
    /// which shut the menu instead of choosing from it. The shell's primary
    /// tab switcher was dead to the one gesture everybody tries first.
    ///
    /// `above` is measured from the bar row's TOP edge, so the first row up is
    /// the LAST tab, because the column reads top to bottom with tab 0 first.
    ///
    /// Written against `rowBias` rather than around it: the mapping is one
    /// row-height shifted DOWN by the bias, and the numbers below are that
    /// sentence read off at the boundaries.
    @Test func aTapOnAColumnRowPicksTheRowUnderIt() {
        let row = ShellMetrics.rowHeight
        let bias = ShellMetrics.rowBias
        // The row nearest the bar is the last tab, and it owns the bias band
        // below its own bottom edge as well.
        #expect(ShellGesture.columnRow(above: 1, tabCount: 3) == 2)
        #expect(ShellGesture.columnRow(above: bias, tabCount: 3) == 2)
        #expect(ShellGesture.columnRow(above: row, tabCount: 3) == 2)
        // One row up, once the finger is a bias past that row's bottom edge.
        #expect(ShellGesture.columnRow(above: row + bias, tabCount: 3) == 1)
        #expect(ShellGesture.columnRow(above: 2 * row, tabCount: 3) == 1)
        // The topmost row is tab 0, and it keeps a full row of its own.
        #expect(ShellGesture.columnRow(above: 2 * row + bias, tabCount: 3) == 0)
        #expect(ShellGesture.columnRow(above: 3 * row, tabCount: 3) == 0)
    }

    /// A touch that is not on the column answers nil, which is what leaves the
    /// tap on the BAR the toggle it has always been.
    @Test func aTapOffTheColumnPicksNoRow() {
        let row = ShellMetrics.rowHeight
        let bias = ShellMetrics.rowBias
        // On the bar row itself, or below it.
        #expect(ShellGesture.columnRow(above: 0, tabCount: 3) == nil)
        #expect(ShellGesture.columnRow(above: -20, tabCount: 3) == nil)
        // Past the column's top edge, and past the margin above it.
        #expect(ShellGesture.columnRow(above: 3 * row + bias, tabCount: 3) == 0)
        #expect(ShellGesture.columnRow(above: 3 * row + bias + 1, tabCount: 3) == nil)
        // A workspace with no tabs has no rows to hit.
        #expect(ShellGesture.columnRow(above: 10, tabCount: 0) == nil)
    }

    /// The selected row sits BELOW the fingertip rather than under it, and by
    /// a quarter of a row.
    ///
    /// The owner's complaint about occlusion: a thumb covers the 44-point row
    /// it is on, so the row you are choosing is the one you cannot see.
    /// Asserted as the property rather than as a table — for every point of a
    /// column, the row chosen is the drawn row at the fingertip or the one
    /// below it, and never the one above.
    ///
    /// The bound in the other direction is what keeps the bias honest as a
    /// bias rather than as an off-by-one: a finger in the middle 33 points of
    /// a row always selects that row, so a deliberate aim is never overridden.
    @Test func theChosenRowSitsBelowTheFingerAndNeverAboveIt() {
        let row = ShellMetrics.rowHeight
        let bias = ShellMetrics.rowBias
        let tabs = 4
        var below = 0
        for step in stride(from: CGFloat(1), through: row * CGFloat(tabs), by: 1) {
            // The row the column actually DRAWS at this height, counting down
            // from the bar the way the column is laid out.
            let drawn = tabs - 1 - min(tabs - 1, Int((step - 0.0001) / row))
            let chosen = ShellGesture.columnRow(above: step, tabCount: tabs)
            #expect(chosen == drawn || chosen == drawn + 1, "at \(step) it may only be that row or the one below it")
            if chosen == drawn + 1 { below += 1 }
            // The middle of any row selects that row, whatever the bias is.
            let intoRow = step.truncatingRemainder(dividingBy: row)
            if intoRow > bias && intoRow <= row {
                #expect(chosen == drawn, "a finger clear of the bias band selects the row it is on")
            }
        }
        #expect(below > 0, "the bias has to actually move the answer somewhere")
        // And the shape of where it moves it: a finger a point above a row
        // boundary is still choosing the row below the boundary, which is the
        // row below the fingertip.
        #expect(ShellGesture.columnRow(above: row + 1, tabCount: tabs) == tabs - 1)
        #expect(ShellGesture.columnRow(above: row + bias, tabCount: tabs) == tabs - 2)
    }

    /// The release reads it: a touch with a row under it LANDS, and one with
    /// none still toggles.
    @Test func aTapWithARowUnderItLandsRatherThanTogglingTheColumn() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)
        #expect(fleet.barRelease(axis: nil, dx: 0, up: 0, at: at, row: 2) == .land(tab: 2))
        #expect(fleet.barRelease(axis: nil, dx: 0, up: 0, at: at, row: nil) == .toggleColumn)
    }

    // MARK: - The column

    /// **The row a DRAG chooses is the row under the finger, wherever the drag
    /// started.** The owner's complaint 3, and the one the old mapping got
    /// wrong by exactly one row.
    ///
    /// This used to be `columnSelection(up:tabCount:)` —
    /// `tabCount - ceil(up / rowHeight)`, a pure delta with no idea where the
    /// finger had been put down. Write `d` for how far below the column's
    /// bottom edge the touch landed and the two mappings agree only at
    /// `d == 0`; at `d == 44` the delta one sat a full row above the finger
    /// for the whole gesture. The old formula is written out below as the
    /// counter-example, so that this test says what changed and not merely
    /// what is.
    ///
    /// The finger is held at ONE height above the bar while the drag's start
    /// is moved down through the bar row, which is the only way to state the
    /// bug: the same finger, in the same place, over the same drawn row, used
    /// to choose three different tabs.
    @Test func theChosenRowFollowsTheFingerAndNotHowFarItHasTravelled() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        // 60 points above the bar's top edge is inside the MIDDLE row of a
        // three-tab column, which is tab 1.
        let above: CGFloat = 60
        let row = ShellGesture.columnRow(above: above, tabCount: 3)
        #expect(row == 1)

        // The delta mapping that used to answer this, kept as the
        // counter-example it is.
        func oldDeltaMapping(up: CGFloat) -> Int? {
            guard up >= ShellMetrics.openMin else { return nil }
            let steps = min(3, Int((up / ShellMetrics.rowHeight).rounded(.up)))
            return steps > 0 ? 3 - steps : nil
        }

        var oldAnswers: Set<Int?> = []
        for d in stride(from: CGFloat(0), through: ShellMetrics.barRow, by: 11) {
            // The same fingertip, reached from a touch that started `d` points
            // lower down the bar row.
            let up = above + d
            #expect(
                fleet.barRelease(axis: .vertical, dx: 0, up: up, at: at, row: row)
                    == .land(tab: 1),
                "the finger is over tab 1's row, so a release chooses tab 1")
            oldAnswers.insert(oldDeltaMapping(up: up))
        }
        #expect(
            oldAnswers.count > 1,
            "the mapping this replaced gave different rows for one fingertip, which is the bug")
        #expect(oldAnswers.contains(0), "and at the bottom of the bar it was a whole row out")
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

    /// The finger still chooses, even though the column no longer unrolls
    /// under it.
    ///
    /// This is the half of the original behaviour worth keeping: one
    /// continuous drag still walks the tabs and carries on into the overview.
    /// What changed is which number it walks — the finger's height above the
    /// bar rather than the distance it has come.
    ///
    /// The `openMin` gate is the one thing still read off the lift, and it is
    /// not a mapping: it is the line between a bar you touched and moved a
    /// little, which must cost nothing, and a tab you chose.
    @Test func theSelectionStillFollowsTheFingerThroughAnOpenColumn() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        func release(up: CGFloat, above: CGFloat) -> ShellRelease {
            fleet.barRelease(
                axis: .vertical, dx: 0, up: up, at: at,
                row: ShellGesture.columnRow(above: above, tabCount: 3))
        }
        // Under the open threshold nothing is chosen, however clearly the
        // finger is over a row.
        #expect(release(up: ShellMetrics.openMin - 0.1, above: 40) == .abandon)
        #expect(release(up: ShellMetrics.openMin, above: 40) == .land(tab: 2))
        #expect(release(up: 60, above: ShellMetrics.rowHeight + 20) == .land(tab: 1))
        #expect(release(up: 120, above: ShellMetrics.rowHeight * 2 + 20) == .land(tab: 0))
        // Above the column there is no row to choose, and the release costs
        // nothing rather than landing on the top tab by default. The menu is
        // not even drawn there — `ShellRootView.menuShouldShow` takes it off
        // the screen the moment the page starts to rise — so landing would be
        // choosing off a list that is not on the screen.
        #expect(release(up: 200, above: 200) == .abandon)
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

    /// **A finished turn sorts on the top rung while drawing the rung below
    /// it**, and those are two different questions about the same tab.
    ///
    /// This is the invariant the move from `ShellMark` to `GlanceMark` was
    /// most able to lose quietly. `done` draws `.toReview` — the owner ruled
    /// that it is the review tier — so a `precedence` that asked the MARK
    /// which rung to use would put every finished agent on the diff rung, one
    /// below where it has always sorted, and the overview would reorder itself
    /// under a person for no reason they could see. It asks
    /// `wantsAttention` instead, which is `AgentActivity`'s own answer and has
    /// been this app's single definition of "interrupt someone" since before
    /// the glance vocabulary existed.
    @Test func aFinishedTurnSortsWithTheBlockedOnesRatherThanWithTheDiffs() {
        let done = ShellWorkspace(id: "d", name: "d", tabs: [Self.tab("d0", .done)])
        #expect(done.tabs[0].mark.attention == .toReview, "done is the review tier, and draws it")
        #expect(done.precedence == .needsYou, "and it still sorts where a person is wanted")

        // The other half, and the reason the rung below did not widen: a diff
        // draws the SAME ring and must not be lifted by it.
        let diff = ShellWorkspace(id: "f", name: "f", tabs: [Self.tab("f0", .unreadDiff)])
        #expect(diff.tabs[0].mark.attention == .toReview)
        #expect(diff.precedence == .unreadDiff, "a diff is not an agent asking for you")

        // And the order they land in, which is what a person actually sees.
        let fleet = ShellFleet(workspaces: [
            ShellWorkspace(id: "0", name: "quiet", tabs: [Self.tab("x", .idle)]),
            diff,
            done,
            ShellWorkspace(id: "3", name: "blocked", tabs: [Self.tab("x", .needsYou)]),
        ])
        #expect(
            fleet.overviewOrder().map { fleet.workspaces[$0].id } == ["d", "3", "f", "0"],
            "done and blocked share the top rung, in fleet order; then the diff; then the quiet one")
    }

    /// An idle agent is not a stale one, and a workspace full of idle agents
    /// is not a workspace we have stopped hearing from.
    ///
    /// `allStale` reads the link axis now rather than a case called `stale`,
    /// and the two are easy to conflate: `idle` and `stale` were ONE case
    /// under `ShellMark` for every purpose except the one this rung is about.
    @Test func aWorkspaceOfIdleAgentsIsNotAWorkspaceGoneQuiet() {
        let idle = ShellWorkspace(
            id: "i", name: "i", tabs: [Self.tab("i0", .idle), Self.tab("i1", .idle)])
        #expect(idle.precedence == .working, "idle is a live answer; it just is not a busy one")
        let gone = ShellWorkspace(
            id: "g", name: "g", tabs: [Self.tab("g0", .idle), Self.tab("g1", .stale)])
        #expect(gone.precedence == .working, "one tab we have not heard from is not the workspace")
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

    // MARK: - Hidden worktrees, and other runners

    /// A hidden worktree leaves the grid's main list and turns up in the
    /// section that reveals it — it does not vanish, and it does not stay
    /// where it was.
    ///
    /// Both halves asserted, because each fails on its own: a filter with no
    /// section loses the way back from hiding, and a section that also left
    /// the card in place would draw it twice.
    @Test func hidingTakesAWorkspaceOutOfTheGridAndIntoItsOwnSection() {
        let fleet = ShellFleet(workspaces: [
            ShellWorkspace(id: "0", name: "shown-a", tabs: [Self.tab("x", .working)]),
            ShellWorkspace(
                id: "1", name: "put-away", isHidden: true, tabs: [Self.tab("x", .needsYou)]),
            ShellWorkspace(id: "2", name: "shown-b", tabs: [Self.tab("x", .working)]),
        ])
        #expect(fleet.overviewOrder() == [0, 2])
        #expect(fleet.hiddenOrder() == [1])
        #expect(
            fleet.workspaces.count == 3,
            "hiding is a view preference; the workspace keeps its place in the fleet")
    }

    /// Hiding must not reorder what is still showing. A hidden workspace that
    /// outranks everything is exactly the case where a shared sort and a
    /// separate one diverge.
    @Test func theHiddenSectionUsesTheSameSortAndDoesNotDisturbTheRest() {
        let fleet = ShellFleet(workspaces: [
            ShellWorkspace(id: "0", name: "working", tabs: [Self.tab("x", .working)]),
            ShellWorkspace(
                id: "1", name: "loud-but-hidden", isHidden: true,
                tabs: [Self.tab("x", .needsYou)]),
            ShellWorkspace(id: "2", name: "diff", tabs: [Self.tab("x", .unreadDiff)]),
            ShellWorkspace(
                id: "3", name: "quiet-hidden", isHidden: true, tabs: [Self.tab("x", .working)]),
        ])
        #expect(fleet.overviewOrder() == [2, 0])
        #expect(fleet.hiddenOrder() == [1, 3], "the section sorts by precedence too")
    }

    /// A worktree you hid is still a worktree you can ask for by name.
    @Test func searchReachesIntoTheHiddenSection() {
        let fleet = ShellFleet(workspaces: [
            ShellWorkspace(id: "0", name: "feat/retries", tabs: [Self.tab("x", .working)]),
            ShellWorkspace(
                id: "1", name: "fix/RETRY-storm", isHidden: true, tabs: [Self.tab("x", .working)]),
        ])
        #expect(fleet.overviewOrder(matching: "retr") == [0])
        #expect(fleet.hiddenOrder(matching: "retr") == [1])
        #expect(fleet.hiddenOrder(matching: "deps") == [])
    }

    /// Another runner's group sorts its cards the way the live fleet sorts
    /// its own, and leaves that runner's hidden worktrees out entirely.
    @Test func aServerGroupSortsLikeTheFleetAndDropsHiddenWorktrees() {
        let group = ShellServerGroup(
            id: "r1", name: "gpu-box-2",
            workspaces: [
                ShellWorkspace(id: "a", name: "working", tabs: [Self.tab("x", .working)]),
                ShellWorkspace(id: "b", name: "needs", tabs: [Self.tab("x", .needsYou)]),
                ShellWorkspace(
                    id: "c", name: "put-away", isHidden: true, tabs: [Self.tab("x", .needsYou)]),
            ])
        #expect(group.order() == [1, 0])
        #expect(group.order(matching: "work") == [0])
    }

    /// The runner you were on ten minutes ago comes above the one you last
    /// opened in March, and a runner this app has never reached comes last.
    @Test func serverGroupsSortByWhenTheyWereLastSeen() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        func group(_ name: String, _ seen: Date?) -> ShellServerGroup {
            ShellServerGroup(
                id: name, name: name, lastSeen: seen,
                workspaces: [ShellWorkspace(id: "w", name: "w", tabs: [Self.tab("x", .working)])])
        }
        let arranged = ShellServerGroup.arrange([
            group("march", now.addingTimeInterval(-90 * 86_400)),
            group("never", nil),
            group("recent", now.addingTimeInterval(-600)),
        ])
        #expect(arranged.map(\.name) == ["recent", "march", "never"])
    }

    /// A header standing over no cards reads as a runner that has gone empty.
    /// A search nothing on that runner matches must remove the header too.
    @Test func aServerGroupWithNothingToShowIsNotDrawn() {
        let full = ShellServerGroup(
            id: "r1", name: "gpu-box-2",
            workspaces: [
                ShellWorkspace(id: "a", name: "feat/queue", tabs: [Self.tab("x", .working)])
            ])
        let allHidden = ShellServerGroup(
            id: "r2", name: "eu-runner-1",
            workspaces: [
                ShellWorkspace(
                    id: "b", name: "feat/queue", isHidden: true, tabs: [Self.tab("x", .working)])
            ])
        #expect(ShellServerGroup.arrange([full, allHidden]).map(\.name) == ["gpu-box-2"])
        #expect(ShellServerGroup.arrange([full], matching: "queue").count == 1)
        #expect(ShellServerGroup.arrange([full], matching: "nothing").isEmpty)
    }

    // MARK: - The other runners' worktrees, cached

    /// A scratch defaults suite, so a test never writes into the app's own.
    private func scratchDefaults(_ name: String = UUID().uuidString) -> UserDefaults {
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    private func directory(
        _ runner: String, label: String, at seen: Date, workspaces: [String],
        hidden: [String] = [], mark: String = "working"
    ) -> RunnerDirectory {
        RunnerDirectory(
            runner: runner, label: label, seenAt: seen,
            workspaces: (workspaces + hidden).map { name in
                RunnerDirectory.Workspace(
                    id: "\(runner)-\(name)", name: name, isHidden: hidden.contains(name),
                    tabs: [RunnerDirectory.Tab(title: "Diff", mark: mark)], tail: ["$ ▌"])
            })
    }

    /// The newest answer about a runner REPLACES the previous one, and leaves
    /// every other runner's alone.
    ///
    /// Both halves fail differently and both have to hold: appending would
    /// grow the file by an entry every three seconds, and unioning would
    /// resurrect a worktree somebody removed.
    @Test func recordingOneRunnerReplacesItsEntryAndKeepsTheOthers() {
        let defaults = scratchDefaults()
        let then = Date(timeIntervalSince1970: 1_000)
        RunnerDirectoryStore.record(
            directory("a", label: "gpu-box-2", at: then, workspaces: ["one", "two"]),
            in: defaults)
        RunnerDirectoryStore.record(
            directory("b", label: "eu-runner-1", at: then, workspaces: ["far"]), in: defaults)
        RunnerDirectoryStore.record(
            directory("a", label: "gpu-box-2", at: then + 60, workspaces: ["one"]), in: defaults)

        let all = RunnerDirectoryStore.read(from: defaults)
        #expect(all.count == 2, "a third record about a known runner is not a third entry")
        let a = all.first { $0.runner == "a" }
        #expect(a?.workspaces.map(\.name) == ["one"], "the removed worktree stayed removed")
        #expect(a?.seenAt == then + 60)
        #expect(all.first { $0.runner == "b" }?.workspaces.map(\.name) == ["far"])
    }

    /// A runner somebody deleted stops turning up in the grid.
    @Test func forgettingDropsRunnersThatAreNoLongerKnown() {
        let defaults = scratchDefaults()
        let then = Date(timeIntervalSince1970: 1_000)
        RunnerDirectoryStore.record(directory("a", label: "a", at: then, workspaces: ["w"]), in: defaults)
        RunnerDirectoryStore.record(directory("b", label: "b", at: then, workspaces: ["w"]), in: defaults)
        RunnerDirectoryStore.forget(runners: ["a"], in: defaults)
        #expect(RunnerDirectoryStore.read(from: defaults).map(\.runner) == ["a"])
    }

    /// **Blocked holds at any age; working does not.**
    ///
    /// `GlanceMark.Link` states the rule this follows — "decay applies only to
    /// claims about the present. Blocked and to-review hold at any age;
    /// working and idle go dashed" — so a cached card is not uniformly greyed
    /// out. An agent that was waiting on you when this runner was last seen is
    /// still waiting on you, and that is the one thing worth crossing a runner
    /// for.
    @Test func aCachedRunnersMarksDecayOnlyWhereTheyAreClaimsAboutNow() {
        let blocked = RunnerDirectory.decayed("needsYou")
        #expect(blocked.mark.attention == .needsYou)
        #expect(blocked.mark.link == .live, "blocked is latched; it does not go dashed with age")
        #expect(blocked.wantsAttention)

        let diff = RunnerDirectory.decayed("unreadDiff")
        #expect(diff.mark.attention == .toReview)
        #expect(diff.mark.link == .live, "nobody has read it, and time passing does not read it")
        #expect(
            !diff.wantsAttention,
            "a diff is not an agent asking for you; only the rung below is its own")

        // The word that did not exist before the `GlanceMark` migration, and
        // the reason it had to: a finished turn and an unread diff both draw
        // the review ring, and only one of them sorts on the top rung.
        let done = RunnerDirectory.decayed("done")
        #expect(done.mark.attention == .toReview)
        #expect(done.mark.link == .live, "done is latched, exactly as blocked is")
        #expect(done.wantsAttention, "a remembered finished turn still wants you")

        for word in ["working", "idle", "stale", "a-word-from-a-later-build"] {
            let aged = RunnerDirectory.decayed(word)
            #expect(aged.mark.link == .broken, "\(word) is a claim about now, and this is not now")
            #expect(aged.mark.attention == .quiet)
            #expect(!aged.wantsAttention)
            #expect(
                aged.mark.core == nil,
                "\(word): not being told what it is doing is not the same as being told it is idle")
        }

        // The round trip, so the two halves cannot drift into writing a word
        // the reader does not know. Spelled as marks in and marks out rather
        // than as `!= nil`, which would be vacuous.
        let trips: [(GlanceMark, Bool)] = [
            (GlanceMark(attention: .needsYou, core: .atAPrompt), true),
            (GlanceMark(attention: .toReview, core: .atAPrompt), true),  // done
            (GlanceMark(attention: .toReview, core: nil), false),  // a diff
        ]
        for (mark, wants) in trips {
            let back = RunnerDirectory.decayed(RunnerDirectory.word(for: mark))
            #expect(back.mark == mark, "\(RunnerDirectory.word(for: mark)) did not survive a trip")
            #expect(back.wantsAttention == wants)
        }
        // And the ones that are MEANT to decay, which is the other half of the
        // rule and would be silently lost if only the survivors were checked.
        for mark in [
            GlanceMark(attention: .quiet, core: .producing),
            GlanceMark(attention: .quiet, core: .atAPrompt),
        ] {
            #expect(RunnerDirectory.decayed(RunnerDirectory.word(for: mark)).mark.link == .broken)
        }
    }

    /// The cache becomes a group the grid can draw: the runner's label on
    /// every card, its worktrees in order, and its hidden ones left out.
    @Test func aCachedRunnerBecomesAGroupTheGridCanDraw() {
        let seen = Date(timeIntervalSince1970: 1_000)
        let group = directory(
            "a", label: "gpu-box-2", at: seen, workspaces: ["feat/queue"], hidden: ["old"],
            mark: "needsYou"
        ).group()
        #expect(group.id == "a", "a tap has to name the runner, not its label")
        #expect(group.name == "gpu-box-2")
        #expect(group.lastSeen == seen)
        #expect(group.order().count == 1, "the hidden one is not drawn")
        #expect(group.workspaces[0].server == "gpu-box-2", "a cached card names its runner")
        #expect(group.workspaces[0].tabs[0].mark.attention == .needsYou)
        #expect(
            group.workspaces[0].tabs[0].wantsAttention,
            "the sort flag has to survive the cache, or a remembered blocked workspace sinks")
    }

    // MARK: - Momentum

    /// The throw distance is a scroll view's own, to the point.
    ///
    /// The talk's projection function with `UIScrollView.DecelerationRate.normal`,
    /// which is what makes a flick in this shell travel exactly as far as a
    /// flick in every other scroll view on the phone: a thousand points per
    /// second coasts 499 points, and there is nothing new to learn about how
    /// far a throw goes.
    ///
    /// **`TerminalScrollPhysics.projection(of:decelerationRate:)` in
    /// `TerminalView.swift` is the same formula and the same 0.998**, and
    /// these numbers are the join between them: that one is UIKit and
    /// iOS-only, this one has to run under `swift test` with no simulator, so
    /// the pair cannot be one function today and the numbers are what stop
    /// them drifting apart in silence.
    @Test func theProjectionIsAScrollViewsOwnThrowDistance() {
        #expect(ShellGesture.decelerationRate == 0.998)
        #expect(abs(ShellGesture.project(velocity: 1000) - 499) < 0.01)
        #expect(abs(ShellGesture.project(velocity: 2000) - 998) < 0.01)
        #expect(abs(ShellGesture.project(velocity: -1000) + 499) < 0.01)
        #expect(ShellGesture.project(velocity: 0) == 0)
        // A rate that is not a decay is not a projection, and answering zero
        // is the only thing that cannot make a release worse.
        #expect(ShellGesture.project(velocity: 1000, decelerationRate: 1) == 0)
        #expect(ShellGesture.project(velocity: 1000, decelerationRate: 0) == 0)
        // Nothing moving projects nowhere, so every default in this file is
        // the old behaviour exactly.
        #expect(ShellGesture.projected(40, velocity: 0) == 40)
    }

    /// **A flick up from the bar reaches the overview even though the thumb
    /// lifted over a menu row.** The owner's complaint 4.
    ///
    /// The bug this pins is the talk's PIP example reproduced as the *before*
    /// case: *"the issue here is that we're only looking at position, we're
    /// completely ignoring the momentum."* The overview used to be reachable
    /// only by dragging a full 76 points past the last row and stopping
    /// there — so a flick, which is how anybody who has used the app switcher
    /// asks for a grid of cards, landed on whichever row the thumb happened
    /// to be passing.
    ///
    /// The negative controls are the point of the test rather than a
    /// footnote: the same finger in the same place, released with no momentum
    /// and with a little, still lands on the row it is over. A projection
    /// that escaped from a standstill would have replaced a bug where you
    /// could not leave with one where you could not stay.
    @Test func aFlickUpFromOverAMenuRowEscapesToTheOverview() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        // Sixty points up is inside the middle row of a three-tab column, and
        // 148 points short of the overview.
        let up: CGFloat = 60
        let row = ShellGesture.columnRow(above: up, tabCount: 3)
        #expect(row == 1)
        func release(_ velocity: CGFloat, dx: CGFloat = 0) -> ShellRelease {
            fleet.barRelease(
                axis: .vertical, dx: dx, up: up, at: at, row: row, dxVelocity: dx == 0 ? 0 : -3000,
                upVelocity: velocity)
        }
        #expect(release(1500) == .openOverview, "a flick is going somewhere the finger has not reached")
        #expect(release(0) == .land(tab: 1), "a finger that stopped chose the row it stopped on")
        #expect(release(200) == .land(tab: 1), "and one still drifting has not asked to leave")
        // A flick that is ALSO sideways still does not carry, because a carry
        // is a page being moved between cells and there is no page in your
        // hand yet — the column is what the finger is in. `pageIsHeld` reads
        // the real lift and not the thrown one, and this is why.
        #expect(release(1500, dx: -200) == .openOverview)
    }

    /// How little of a lift a flick can escape from, stated as a number so it
    /// can be argued with.
    ///
    /// The projection is linear in velocity, so the velocity that escapes
    /// from a given lift is arithmetic rather than a taste: from 60 points up
    /// a three-tab column, 148 points of projection are needed, and 148
    /// points is 297 points per second. That is a thumb still moving rather
    /// than a hard flick, and it is the honest cost of using a scroll view's
    /// own deceleration rate — the same rate makes a 1000 pt/s flick coast
    /// 499 points, and the two cannot be tuned apart.
    @Test func theVelocityThatEscapesFromALiftIsAKnownNumber() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        let up: CGFloat = 60
        let needed = ShellGesture.columnFull(tabCount: 3) + ShellMetrics.overRun - up
        let velocity = needed * 1000 / ShellGesture.project(velocity: 1000)
        #expect(abs(velocity - 296.6) < 0.5)
        func release(_ v: CGFloat) -> ShellRelease {
            fleet.barRelease(
                axis: .vertical, dx: 0, up: up, at: at,
                row: ShellGesture.columnRow(above: up, tabCount: 3), upVelocity: v)
        }
        #expect(release(velocity + 1) == .openOverview)
        #expect(release(velocity - 1) == .land(tab: 1))
    }

    /// **A short fast flick across a pane turns the page.** The sideways half
    /// of the same defect, which the owner did not report and which the bare
    /// 70-point threshold made certain.
    ///
    /// Forty points is a flick anybody would make and is nowhere near
    /// `pageCommit`, so a terminal was a pane you could only leave by a long
    /// laborious drag — the talk's own counter-example: *"those same swipes
    /// wouldn't get you very far… you'd have to do these long, laborious
    /// swipes."*
    @Test func aShortFlickAcrossAPaneTurnsThePageAndADeliberateDragDoesNot() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)
        let next = ShellStep(position: ShellPosition(workspace: 1, tab: 1), crossesWorkspace: false)
        #expect(
            fleet.contentRelease(axis: .horizontal, dx: -40, at: at, dxVelocity: -600)
                == .commit(next))
        #expect(
            fleet.contentRelease(axis: .horizontal, dx: -40, at: at, dxVelocity: 0) == .springBack,
            "the same forty points, placed rather than thrown, is still nothing")
        // And the other direction of the same rule: the velocity that commits
        // from forty points is 60 points per second, which is a thumb that
        // has not quite stopped. The number is here to be looked at.
        #expect(
            fleet.contentRelease(axis: .horizontal, dx: -40, at: at, dxVelocity: -61)
                == .commit(next))
        #expect(
            fleet.contentRelease(axis: .horizontal, dx: -40, at: at, dxVelocity: -59)
                == .springBack)
        // A vertical lock on the content still resolves to nothing, however
        // fast it was going: the pane owns that gesture.
        #expect(
            fleet.contentRelease(axis: .vertical, dx: -200, at: at, dxVelocity: -3000)
                == .springBack)
    }

    /// The direction comes off the THROW, not off the translation.
    ///
    /// A drag one way that is flicked back the other way at the last moment
    /// is asking to go where it is heading. Reading `commits` off the
    /// projection and `direction` off the raw `dx` would turn the page
    /// backwards, which is a worse answer than the spring-back it replaced —
    /// so both come off one number.
    @Test func aDragFlickedBackTheOtherWayTurnsThePageTheWayItIsHeaded() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)
        #expect(
            fleet.contentRelease(axis: .horizontal, dx: 30, at: at, dxVelocity: -1500)
                == .commit(
                    ShellStep(
                        position: ShellPosition(workspace: 1, tab: 1), crossesWorkspace: false)),
            "dragged 30 points right, thrown 718 points left")
    }

    /// The bar's own page turn projects too, and a flick along it crosses
    /// workspaces.
    @Test func aFlickAlongTheBarCrossesWorkspaces() {
        let fleet = Self.crossing()
        let middle = ShellPosition(workspace: 1, tab: 2)
        #expect(
            fleet.barRelease(axis: .horizontal, dx: -30, up: 0, at: middle, dxVelocity: -1500)
                == .commit(
                    ShellStep(
                        position: ShellPosition(workspace: 2, tab: 0), crossesWorkspace: true)))
        #expect(
            fleet.barRelease(axis: .horizontal, dx: -30, up: 0, at: middle) == .springBack,
            "and thirty points placed deliberately is still thirty points")
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

    ///
    /// A drag that started at the bar row's TOP edge, so the finger's height
    /// above the bar and the distance it has travelled are the same number —
    /// which is the one case the mapping this replaced also got right.
    @Test func theBarsVerticalArmAbandonsLandsOrOpensTheOverview() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        func release(up: CGFloat) -> ShellRelease {
            fleet.barRelease(
                axis: .vertical, dx: 0, up: up, at: at,
                row: ShellGesture.columnRow(above: up, tabCount: 3))
        }
        #expect(release(up: 10) == .abandon)
        // The menu reads top to bottom, so the row the first points of lift
        // put under the thumb — the one nearest the bar — is the LAST tab,
        // and further lift walks upward toward tab 0.
        #expect(release(up: ShellMetrics.openMin) == .land(tab: 2))
        #expect(release(up: ShellMetrics.rowHeight + ShellMetrics.rowBias) == .land(tab: 1))
        #expect(
            release(up: ShellMetrics.rowHeight * 3) == .land(tab: 0),
            "a lift that fills the column has walked all the way up to tab 0")
        #expect(release(up: ShellMetrics.rowHeight * 3 + ShellMetrics.overRun) == .openOverview)
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

    /// **A thumb that ARCS sideways while it is choosing a column row still
    /// chooses the row**, and one that decisively swipes sideways does not.
    ///
    /// This test used to make its point with two hundred points of "wander",
    /// which it could, because the axis was locked and the vertical arm threw
    /// away `dx` entirely below `pageIsHeld`. Two hundred points is not a
    /// wander. It is a swipe, and under a redirectable bar it is read as one
    /// — so the assertion has to be made with a number a thumb can actually
    /// produce, and the second half of the test is the number it cannot.
    ///
    /// The arc is what makes the whole change safe, and it is geometry rather
    /// than taste: a thumb pivots about the palm at roughly 100mm, so
    /// sweeping the 132 points that open a three-tab column deviates about 24
    /// points sideways — a ratio of 0.18 against a `ShellMetrics.redirect` of
    /// 1.4, which is not a near miss. An arc's deviation is by definition
    /// small compared to the travel that draws it, and that is the property
    /// being leaned on.
    ///
    /// Driven as a PATH rather than as an endpoint, because that is the only
    /// way to state it now: the answer depends on how the finger got there.
    @Test func aSidewaysArcWhileChoosingARowStillChoosesTheRow() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        // A thumb sweeping to the middle of a three-tab column, arcing 24
        // points across on the way — leaning further the higher it goes,
        // which is what a pivot does.
        let arc: [(dx: CGFloat, up: CGFloat)] = (1...60).map { i in
            let t = CGFloat(i) / 60
            return (dx: -24 * t * t, up: 76 * t)
        }
        let drawn = drive(arc, tabCount: 3)
        #expect(drawn.frame.axis == .vertical, "an arc took the gesture off the column")
        #expect(drawn.flips == 0)
        let above = ShellMetrics.rowHeight + ShellMetrics.rowBias
        #expect(
            fleet.barRelease(
                axis: drawn.frame.axis, dx: drawn.frame.sideways, up: drawn.frame.lift, at: at,
                row: ShellGesture.columnRow(above: above, tabCount: 3))
                == .land(tab: 1))
        // And an arc released as a FLICK is still not a page turn: below
        // `pageIsHeld` the vertical arm reads no sideways at all, however
        // hard the thumb was moving when it left.
        #expect(
            fleet.barRelease(
                axis: drawn.frame.axis, dx: drawn.frame.sideways, up: drawn.frame.lift, at: at,
                row: ShellGesture.columnRow(above: above, tabCount: 3), dxVelocity: -3000)
                == .land(tab: 1))
        #expect(
            fleet.barRelease(axis: .vertical, dx: 200, up: 10, at: at, row: 2) == .abandon,
            "and below the open threshold a release still costs nothing")

        // The other side of the same line, and the reason it is a line: 200
        // points sideways off the same 76-point lift is not an arc, and it
        // now means what it looks like.
        let swiped = drive(arc + leg(from: (-24, 76), to: (-200, 76)), tabCount: 3)
        #expect(swiped.frame.axis == .horizontal)
        #expect(
            fleet.barRelease(
                axis: swiped.frame.axis, dx: swiped.frame.sideways, up: 0, at: at)
                == .commit(
                    ShellStep(
                        position: ShellPosition(workspace: 2, tab: 0), crossesWorkspace: true)))
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

    /// **A fast upward fling reaches the overview and does NOT carry.** The
    /// owner's report: *"when I fling the workspace up, quite often it
    /// animates the workspace to the n-1th or n+1th grid square… if my fling
    /// is angled too much it picks either the previous or next workspace to
    /// land on, seemingly assuming I already switched to it (clearly I
    /// didn't)."*
    ///
    /// The arc is the same one `aSidewaysArcWhileChoosingARowStillChoosesTheRow`
    /// measures — a thumb pivoting about the palm deviates about 24 points
    /// across the 132 that open a three-tab column, a displacement ratio of
    /// 0.18. **Its VELOCITY ratio at the release is twice that**, because for
    /// `x = -24t²` against `y = 132t` the tangent is `dx/dy = -48t/132`, which
    /// at `t = 1` is 0.36. So a thumb leaving at 3000 points per second
    /// upward is leaving at about 1090 sideways, and 1090 projects 544 points
    /// — eight times the 70 that commits a page turn.
    ///
    /// That is the whole defect: `dx` of 24 could never commit, and the
    /// momentum was mixed into the sideways channel without ever being asked
    /// which way the thumb was actually going.
    @Test func aFastAngledFlingReachesTheOverviewWithoutCarrying() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 0)  // three tabs
        // Past the last row and still climbing: the page is in your hand.
        let up = ShellGesture.columnFull(tabCount: 3) + 8
        #expect(ShellGesture.pageIsHeld(up: up, tabCount: 3))
        let upVelocity: CGFloat = 3000
        let dxVelocity = -0.36 * upVelocity  // the arc's tangent, not its chord
        #expect(abs(ShellGesture.projected(-24, velocity: dxVelocity)) > ShellMetrics.pageCommit,
            "the sideways THROW clears the commit on its own — this is the trap")
        #expect(
            fleet.barRelease(
                axis: .vertical, dx: -24, up: up, at: at,
                dxVelocity: dxVelocity, upVelocity: upVelocity) == .openOverview,
            "an overwhelmingly vertical fling landed the page in a neighbour's cell")
        // The mirror image, so the fix cannot be a sign error.
        #expect(
            fleet.barRelease(
                axis: .vertical, dx: 24, up: up, at: at,
                dxVelocity: -dxVelocity, upVelocity: upVelocity) == .openOverview)
    }

    /// **A lifted page genuinely flicked sideways still carries**, which is
    /// the gesture the `.carry` arm exists for and the thing the fix above
    /// must not have cost.
    ///
    /// Three ways of asking for it, and all three still work:
    /// placed with no momentum at all, flicked from short of the commit, and
    /// flicked from a standstill after the lift has finished. The first is
    /// the one that proves the narrowing is on the MOMENTUM only — `dx` is
    /// never discounted, so a card drawn into the neighbour's cell carries
    /// however it got there.
    @Test func aLiftedPageFlickedSidewaysStillCarries() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 2)
        let up = ShellGesture.columnFull(tabCount: 3) + ShellMetrics.overRun
        let next = ShellRelease.carry(
            ShellStep(position: ShellPosition(workspace: 2, tab: 0), crossesWorkspace: true))
        // Placed: seventy points of drawn card, nothing moving.
        #expect(fleet.barRelease(axis: .vertical, dx: -70, up: up, at: at) == next)
        // Flicked from forty points — short of the commit on translation
        // alone, and carried by the momentum. The thumb has finished rising
        // and is moving across, which is what a deliberate carry IS.
        #expect(
            fleet.barRelease(
                axis: .vertical, dx: -40, up: up, at: at,
                dxVelocity: -1500, upVelocity: 0) == next)
        // And still carried while the thumb is drifting upward, so long as it
        // is going sideways faster: 1500 across against 1000 up is 56° off
        // vertical, past the 54.5° the ratio draws.
        #expect(
            fleet.barRelease(
                axis: .vertical, dx: -40, up: up, at: at,
                dxVelocity: -1500, upVelocity: 1000) == next)
        // The lower branch of the same composition — lifted but not far
        // enough to stay up — flicked sideways is still an ordinary crossing.
        #expect(
            fleet.barRelease(
                axis: .vertical, dx: -40, up: ShellGesture.columnFull(tabCount: 3) + 1, at: at,
                dxVelocity: -1500, upVelocity: 0)
                == .commit(
                    ShellStep(
                        position: ShellPosition(workspace: 2, tab: 0), crossesWorkspace: true)))
    }

    /// **The angle at which a release stops carrying, as a number.**
    ///
    /// `ShellMetrics.redirect` is `tan 54.5°`, so a release whose velocity is
    /// more than 54.46° off vertical has its momentum counted sideways and
    /// one within it does not. Equivalently: the thumb has to be leaving
    /// within 35.54° of horizontal to throw the page into a neighbour's cell.
    /// It is the same angle `lean` uses to take a gesture off the vertical,
    /// which is the point — one ratio, asked once about where the finger has
    /// been and once about where it is going.
    ///
    /// Below the angle the drawn `dx` is all that is left, so the carry
    /// depends on whether the card was actually moved into the cell. Both
    /// sides of THAT are checked too, because a gate that swallowed a
    /// committed translation would be a different bug.
    @Test func theCarryTakesTheMomentumOnlyWithin35PointFiveDegreesOfHorizontal() {
        let fleet = Self.crossing()
        let at = ShellPosition(workspace: 1, tab: 2)
        let up = ShellGesture.columnFull(tabCount: 3) + ShellMetrics.overRun
        let speed: CGFloat = 3000
        let boundary = atan(1 / ShellMetrics.redirect)  // from horizontal
        #expect(abs(boundary * 180 / .pi - 35.5376) < 0.001)
        func release(offVertical degrees: CGFloat, dx: CGFloat) -> ShellRelease {
            let radians = degrees * .pi / 180
            return fleet.barRelease(
                axis: .vertical, dx: dx, up: up, at: at,
                dxVelocity: -speed * sin(radians), upVelocity: speed * cos(radians))
        }
        let carried = ShellRelease.carry(
            ShellStep(position: ShellPosition(workspace: 2, tab: 0), crossesWorkspace: true))
        // Twenty-four points of drawn card — the thumb's arc, which cannot
        // commit on its own. Only the momentum could carry it, and only
        // past the angle.
        #expect(release(offVertical: 54.5, dx: -24) == carried)
        #expect(release(offVertical: 54.4, dx: -24) == .openOverview)
        // The owner's fling is at 20°, nowhere near it.
        #expect(release(offVertical: 20, dx: -24) == .openOverview)
        // Seventy points of drawn card carries at EVERY angle, because the
        // gate is on the momentum and never on the drawing.
        #expect(release(offVertical: 0, dx: -70) == carried)
        #expect(release(offVertical: 20, dx: -70) == carried)
        // A fast angled fling DOWN out of the overview's run has the same
        // lateral shadow, and is refused the same way — `abs` on the vertical
        // is what makes the two symmetric. The row under the thumb wins,
        // which is the answer a gesture that is not escaping should get; the
        // bug would have crossed a workspace on the way down. Eight points
        // past the last row is the TOP of the column, so the row is tab 0 —
        // `columnRow` counts down from the bar and inverts.
        let lowered = ShellGesture.columnFull(tabCount: 3) + 8
        #expect(
            fleet.barRelease(
                axis: .vertical, dx: -24, up: lowered, at: at,
                row: ShellGesture.columnRow(above: lowered, tabCount: 3),
                dxVelocity: -1090, upVelocity: -3000) == .land(tab: 0))
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

    // MARK: - Leaving the overview by hand

    /// **The pull back out is tracked for its whole length**, over exactly the
    /// distance the lift spent putting the page away.
    ///
    /// The point of the assertion is the pair of endpoints and the fact that
    /// there is something in between: what this replaced had no in-between at
    /// all, only a boolean read at the release.
    @Test func thePullOutOfTheOverviewIsTrackedOverTheLiftsOwnRun() {
        #expect(ShellGesture.pullProgress(down: 0) == 0)
        #expect(ShellGesture.pullProgress(down: ShellMetrics.overRun) == 1)
        #expect(ShellGesture.pullProgress(down: ShellMetrics.overRun / 2) == 0.5)
        // Clamped at both ends: a drag that wanders upward moves nothing, and
        // one that keeps going past the display has nothing left to grow.
        #expect(ShellGesture.pullProgress(down: -120) == 0)
        #expect(ShellGesture.pullProgress(down: 400) == 1)
    }

    /// **The release reads momentum**, which is what makes a flick down off
    /// the top of the grid mean the same thing as a deliberate pull.
    ///
    /// Two gestures that travelled the SAME distance and resolve differently,
    /// which is the whole of the projection: 20 points placed and let go of
    /// stays, 20 points still moving at 600 points a second leaves.
    @Test func aFlickOutOfTheGridLeavesAndAPlacedTwentyPointsDoesNot() {
        #expect(!ShellGesture.pullCommits(down: 20))
        #expect(ShellGesture.pullCommits(down: 20, velocity: 600))
        // And the deliberate pull is unchanged: the threshold is still the
        // same 40 points it was when it was read off the translation alone,
        // so nothing anybody had learned about this gesture stopped being
        // true.
        #expect(!ShellGesture.pullCommits(down: 39.9))
        #expect(ShellGesture.pullCommits(down: 40))
    }

    /// The commit sits just past the half way point of the motion the pull
    /// draws, so letting go of a page more than half way home sends it home.
    ///
    /// Stated as a relationship rather than as two numbers, because the two
    /// numbers only mean anything together — a threshold beyond the travel
    /// would be a gesture that tracks all the way and then refuses.
    @Test func lettingGoPastHalfWayHomeGoesHome() {
        #expect(ShellMetrics.pullDismiss > ShellMetrics.overRun / 2)
        #expect(ShellMetrics.pullDismiss < ShellMetrics.overRun)
        #expect(ShellGesture.pullCommits(down: ShellMetrics.overRun * 0.6))
        #expect(!ShellGesture.pullCommits(down: ShellMetrics.overRun * 0.4))
    }
}
