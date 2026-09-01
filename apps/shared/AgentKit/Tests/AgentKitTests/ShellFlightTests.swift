import Foundation
import Testing

@testable import AgentKit

/// The flying page's geometry, checked without a simulator.
///
/// The sibling of `ShellNavigationTests`, and it exists for the same reason:
/// the iOS target has no unit test bundle, so a transform that lives inside a
/// `View` can only ever be checked by a person swiping at it. What is asserted
/// here is the set of things that have each been wrong once and each looked,
/// on a screen, like the same complaint — "it vanishes into the hole".
///
/// The assertions are about SHAPE rather than about taste. Nobody here checks
/// that 0.82 is the right held scale; what is checked is that the page is a
/// page while a finger is on it, that it is exactly the cell by the end of the
/// flight, and that every one of the four numbers is continuous in between —
/// because the bugs were steps, not values.
struct ShellFlightTests {
    /// A 402 × 874 display, which is the phone the shell was tuned on, at the
    /// origin so that "where the page is drawn" and "the offset" are the same
    /// number.
    private let page = CGRect(x: 0, y: 0, width: 402, height: 874)

    /// A 168 × 132 cell, the second in the top row of the grid.
    private let tile = CGRect(x: 210, y: 260, width: 168, height: 132)

    // MARK: - Becoming a card

    /// A page under a finger is still a PAGE, however far it has been lifted.
    ///
    /// The property the owner asked for, and the one the crop used to break:
    /// the whole screen is drawn all the way up, and only the release turns it
    /// into a card. `cropped` is zero for the whole of a lift, so this is the
    /// same as saying the crop cannot start early.
    @Test func aHeldPageIsStillAWholePage() {
        for rise in stride(from: CGFloat(0), through: 400, by: 20) {
            #expect(
                ShellFlight.height(page: page, tile: tile, cropped: 0) == page.height,
                "the page was cropped at a rise of \(rise) with nothing released")
        }
    }

    /// And it is never smaller than `heldScale`, which is the other half of the
    /// same rule: the shrink under a finger stops well short of the cell.
    ///
    /// The page used to shrink all the way to the tile's own width under the
    /// thumb — 168 into 402, 42% — so by the top of a lift you were holding a
    /// thumbnail and the release had nothing left to do.
    @Test func aHeldPageNeverShrinksPastTheHeldScale() {
        for rise in stride(from: CGFloat(0), through: 600, by: 10) {
            let scale = ShellFlight.scale(page: page, tile: tile, rise: rise, cropped: 0)
            #expect(scale <= 1)
            #expect(
                scale >= ShellMotion.heldScale - 0.0001,
                "a rise of \(rise) shrank the page to \(scale), past the held scale")
        }
    }

    /// The shrink is still visibly running at the top of a real lift.
    ///
    /// `liftReach` is twice `overRun` precisely so this is true: normalized by
    /// the 76-point threshold instead, the shrink finishes in the first quarter
    /// of a three-hundred-point drag and then holds still, which is a page that
    /// stopped answering a finger that is still moving.
    @Test func theShrinkIsStillRunningAtTheOverviewThreshold() {
        let atThreshold = ShellFlight.scale(
            page: page, tile: tile, rise: ShellMetrics.overRun, cropped: 0)
        let later = ShellFlight.scale(
            page: page, tile: tile, rise: ShellMetrics.overRun * 1.5, cropped: 0)
        #expect(later < atThreshold, "the held shrink had already finished by the threshold")
        #expect(atThreshold > ShellMotion.heldScale, "the page reached its held size too early")
    }

    /// The ramp is eased rather than linear: the first points of travel cost
    /// less than the last.
    ///
    /// A linear ramp was the specific complaint — a finger's travel is linear
    /// and a page that answers it linearly reads as a slider rather than as an
    /// object being picked up — and the gentle start is also what a thumb that
    /// overshoots a menu needs.
    @Test func theHeldShrinkStartsGently() {
        let first = 1 - ShellFlight.scale(page: page, tile: tile, rise: 20, cropped: 0)
        let second =
            ShellFlight.scale(page: page, tile: tile, rise: 20, cropped: 0)
            - ShellFlight.scale(page: page, tile: tile, rise: 40, cropped: 0)
        #expect(second > first, "the first 20 points shrank the page as much as the next 20")
    }

    // MARK: - Arriving

    /// At the end of the flight the page IS the cell: its width, its height,
    /// its corner and its place.
    ///
    /// All four at once, because the landing is only invisible if every one of
    /// them agrees. A page that arrives at the right place with the wrong crop
    /// is a sliver lying across three rows of the grid.
    @Test func aLandedPageIsExactlyTheCell() {
        let scale = ShellFlight.scale(page: page, tile: tile, rise: 0, cropped: 1)
        #expect(abs(scale * page.width - tile.width) < 0.001)

        let height = ShellFlight.height(page: page, tile: tile, cropped: 1)
        #expect(abs(height * scale - tile.height) < 0.001, "the crop did not close to the cell")

        let radius = ShellFlight.radius(scale: scale, rise: 0, cropped: 1)
        #expect(
            abs(radius * scale - ShellMotion.cardRadius) < 0.001,
            "the corner did not arrive at the card's")

        let offset = ShellFlight.offset(
            page: page, tile: tile, landing: true, scale: scale, rise: 0, anchorX: 201, carryX: 0)
        #expect(abs(offset.width - (tile.minX - page.minX)) < 0.001)
        // The drawn rectangle's bottom, which is what the clip keeps: the page
        // is offset so that its own bottom edge lands on the cell's.
        #expect(abs(page.minY + page.height * scale + offset.height - tile.maxY) < 0.001)
    }

    /// Every number the flight runs on is continuous in `cropped`.
    ///
    /// **This is the regression that matters most, and it is the one a
    /// screenshot cannot show.** All four used to read a BOOLEAN — "is it
    /// flying" — which is set outside the animation that carries the page, so
    /// the page's height and corner jumped at the instant of release and only
    /// then travelled. The fix was to hang all of it off one interpolatable
    /// number; the assertion is that no small step in that number produces a
    /// large step in anything drawn.
    @Test func nothingAboutTheFlightSteps() {
        var previous: (CGFloat, CGFloat, CGFloat, CGFloat)?
        for i in 0...100 {
            let cropped = CGFloat(i) / 100
            let scale = ShellFlight.scale(page: page, tile: tile, rise: 40, cropped: cropped)
            let height = ShellFlight.height(page: page, tile: tile, cropped: cropped)
            let radius = ShellFlight.radius(scale: scale, rise: 40, cropped: cropped)
            let offset = ShellFlight.offset(
                page: page, tile: tile, landing: true, scale: scale, rise: 40, anchorX: 201,
                carryX: 0)
            if let was = previous {
                // A hundredth of the journey, so nothing may move by more than
                // a few points or a few percent of a scale.
                #expect(abs(scale - was.0) < 0.02, "the scale stepped at cropped = \(cropped)")
                #expect(abs(height - was.1) < 20, "the crop stepped at cropped = \(cropped)")
                #expect(abs(radius - was.2) < 5, "the corner stepped at cropped = \(cropped)")
                #expect(abs(offset.height - was.3) < 20, "the page stepped at cropped = \(cropped)")
            }
            previous = (scale, height, radius, offset.height)
        }
    }

    // MARK: - Where the page is while a finger is on it

    /// The pixel under a motionless thumb does not move as the page shrinks.
    ///
    /// The anchor is the whole of it. The page used to shrink about the middle
    /// of the display, so anywhere else the pixels under the finger slid toward
    /// the centre — a card that is not being moved sideways at all appearing to
    /// slide out from under the thumb holding it.
    @Test func theShrinkHoldsTheTouchPointStill() {
        let anchor: CGFloat = 60
        for rise in stride(from: CGFloat(0), through: 300, by: 25) {
            let scale = ShellFlight.scale(page: page, tile: tile, rise: rise, cropped: 0)
            let offset = ShellFlight.offset(
                page: page, tile: tile, landing: false, scale: scale, rise: rise, anchorX: anchor,
                carryX: 0)
            #expect(
                abs(anchor * scale + offset.width - anchor) < 0.001,
                "the page slid sideways under a still thumb at a rise of \(rise)")
        }
    }

    /// The page's bottom edge follows the finger, one point per point.
    ///
    /// The thing under your thumb is the thing your thumb is moving, and it is
    /// the reason the vertical offset carries the shrink as well as the rise.
    @Test func theBottomEdgeTracksTheFinger() {
        for rise in stride(from: CGFloat(0), through: 300, by: 25) {
            let scale = ShellFlight.scale(page: page, tile: tile, rise: rise, cropped: 0)
            let offset = ShellFlight.offset(
                page: page, tile: tile, landing: false, scale: scale, rise: rise, anchorX: 201,
                carryX: 0)
            #expect(
                abs(page.height * scale + offset.height - (page.height - rise)) < 0.001,
                "the page's bottom edge was not where the finger put it at a rise of \(rise)")
        }
    }

    /// A carried lift moves the card one point per point, on top of everything
    /// else the shrink is doing.
    ///
    /// The second horizontal channel, and the assertion is that it is NOT
    /// scaled by the page's own shrink: feeding a held card from the track's
    /// translation would move it by 29 points for a 70-point thumb move.
    @Test func aCarriedPageFollowsTheThumbSideways() {
        let scale = ShellFlight.scale(page: page, tile: tile, rise: 120, cropped: 0)
        let still = ShellFlight.offset(
            page: page, tile: tile, landing: false, scale: scale, rise: 120, anchorX: 201,
            carryX: 0)
        let carried = ShellFlight.offset(
            page: page, tile: tile, landing: false, scale: scale, rise: 120, anchorX: 201,
            carryX: -70)
        #expect(abs((carried.width - still.width) + 70) < 0.001)
    }

    /// The page does not aim at its cell until the finger is off it.
    ///
    /// The shape of the curve the owner called out — up, then suddenly sideways
    /// — comes from treating the destination as a target while the gesture is
    /// still running. With `landing` false, moving the cell to the other side
    /// of the grid must change nothing at all.
    @Test func aHeldPageIgnoresTheCellItIsGoingTo() {
        let far = CGRect(x: 24, y: 900, width: 168, height: 132)
        let scale = ShellFlight.scale(page: page, tile: tile, rise: 100, cropped: 0)
        let near = ShellFlight.offset(
            page: page, tile: tile, landing: false, scale: scale, rise: 100, anchorX: 201,
            carryX: 0)
        let away = ShellFlight.offset(
            page: page, tile: far, landing: false, scale: scale, rise: 100, anchorX: 201, carryX: 0)
        #expect(near == away, "the page drifted toward its destination mid-drag")
    }

    // MARK: - The corner, and the shadow

    /// A page that has not left the display keeps no corner of its own.
    ///
    /// At rest the display itself is what clips the page, so a radius here
    /// would be drawn over a corner that is already round. It is what happens
    /// AFTER the page leaves the glass that has to look like a device.
    @Test func aPageOnTheGlassDrawsNoCornerOfItsOwn() {
        #expect(ShellFlight.radius(scale: 1, rise: 0, cropped: 0) == 0)
    }

    /// And the display's corner is reached over `cardReveal`, not over the
    /// whole run — the corner is not information, it is what says the thing
    /// under your finger has left the glass.
    @Test func theDisplaysCornerArrivesEarly() {
        let scale = ShellFlight.scale(
            page: page, tile: tile, rise: ShellMotion.cardReveal, cropped: 0)
        let radius = ShellFlight.radius(scale: scale, rise: ShellMotion.cardReveal, cropped: 0)
        #expect(abs(radius * scale - ShellMotion.screenCorner) < 0.5)
    }

    /// The shadow never switches off at the moment of release.
    ///
    /// The two halves of the journey are measured by different numbers — the
    /// rise covers the lift and goes to zero the instant the overview is set;
    /// `cropped` covers the flight and is zero for the whole lift — so a shadow
    /// on either one alone would vanish for exactly the frame in which nothing
    /// at all should change.
    @Test func theShadowSurvivesTheRelease() {
        let held = ShellFlight.offGlass(rise: ShellMetrics.overRun, cropped: 0)
        let released = ShellFlight.offGlass(rise: 0, cropped: 0.0001)
        #expect(held == 1, "a page held past the threshold was not fully off the glass")
        #expect(released > 0, "the shadow switched off at the instant of release")
    }

    // MARK: - Nothing to fly to

    /// A grid that has not laid out the current workspace's cell — a search
    /// that filters it out, most obviously — leaves the page a page.
    ///
    /// Not a crash and not a zero: with no cell there is no destination, so the
    /// shrink is whatever the finger asked for and the crop does not happen.
    @Test func aPageWithNoCellSimplyDoesNotFly() {
        let scale = ShellFlight.scale(page: page, tile: nil, rise: 100, cropped: 1)
        let held = ShellFlight.scale(page: page, tile: nil, rise: 100, cropped: 0)
        #expect(scale == held, "the flight scaled the page toward a cell that does not exist")
        #expect(ShellFlight.height(page: page, tile: nil, cropped: 1) == page.height)
        let offset = ShellFlight.offset(
            page: page, tile: nil, landing: true, scale: scale, rise: 100, anchorX: 201, carryX: 0)
        #expect(offset.height == page.height * (1 - scale) - 100, "a landing with no cell moved")
    }

    /// A page measured at nothing — the frame before the geometry reader has
    /// answered — divides by no zeros.
    @Test func anUnmeasuredPageIsInert() {
        let empty = CGRect.zero
        #expect(ShellFlight.scale(page: empty, tile: tile, rise: 100, cropped: 1) == 1)
        #expect(ShellFlight.height(page: empty, tile: tile, cropped: 1) == 0)
        #expect(
            ShellFlight.offset(
                page: empty, tile: tile, landing: true, scale: 1, rise: 100, anchorX: 0, carryX: 0)
                == .zero)
        #expect(ShellFlight.radius(scale: 0, rise: 100, cropped: 1) == 0)
    }

    // MARK: - The way back, under a thumb

    /// **A tracked return begins exactly where the grid was drawing the card
    /// and ends exactly where a finished flight leaves the page.**
    ///
    /// The two ends are the assertion. A pull-down hands over from the grid on
    /// its first frame and hands over to a full-bleed page on its last, and a
    /// blend that did not agree with `offset`'s own two branches at 0 and 1
    /// would put a step at one end or the other — which is the defect this
    /// whole gesture exists to remove, moved to its edges.
    @Test func aTrackedReturnAgreesWithTheFlightAtBothEnds() {
        // At the cell: the same scale the landed page is drawn at.
        let landed = ShellFlight.scale(page: page, tile: tile, rise: 0, cropped: 1)
        #expect(
            ShellFlight.returning(page: page, tile: tile, scale: landed, progress: 0)
                == ShellFlight.offset(
                    page: page, tile: tile, landing: true, scale: landed, rise: 0,
                    anchorX: 0, carryX: 0))
        // At the display: a page at scale 1 sits at the display's own origin,
        // which is where every other resting page in this shell sits.
        #expect(
            ShellFlight.returning(page: page, tile: tile, scale: 1, progress: 1)
                == CGSize(width: 0, height: 0))
    }

    /// And it MOVES in between, monotonically, on both axes.
    ///
    /// The one thing the threshold it replaced could not do. Sampled rather
    /// than reasoned about: each step has to be in the same direction as the
    /// last, so a blend that stalled, doubled back or jumped is a failure
    /// here rather than something a person has to notice on a phone.
    @Test func aTrackedReturnMovesAtEveryPointOfTheDrag() {
        var last: CGSize?
        var steps: [CGFloat] = []
        for step in 0...20 {
            let along = CGFloat(step) / 20
            let scale = ShellFlight.scale(
                page: page, tile: tile, rise: 0, cropped: 1 - along)
            let now = ShellFlight.returning(
                page: page, tile: tile, scale: scale, progress: along)
            if let last {
                #expect(now != last, "the page did not move between \(along) and the step before")
                steps.append(now.height - last.height)
            }
            last = now
        }
        #expect(steps.allSatisfy { $0 < 0 }, "the page stopped rising on its way home")
    }

    /// With no cell to come out of there is still an answer, and it is the
    /// display. A grid that has not laid out yet must not park the page
    /// somewhere it cannot be seen.
    @Test func aReturnWithNoCellIsStillOnTheDisplay() {
        #expect(
            ShellFlight.returning(page: page, tile: nil, scale: 1, progress: 0)
                == CGSize(width: 0, height: 0))
    }
}
