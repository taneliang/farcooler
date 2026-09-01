import SwiftUI

// What the shell MOVES by, and where the flight's arithmetic lives.
//
// The split this file makes is the one `ShellNavigation.swift` already makes
// one line over: that file is what a gesture DECIDES — thresholds a release is
// measured against — and this is what a page LOOKS LIKE while it is on its
// way. Both halves are pure, both are reachable from `swift test`, and neither
// needs a simulator to be wrong in front of you.
//
// It is here rather than beside the views for the reason `ShellNavigation`'s
// own header gives: the iOS target has no unit test bundle, only UI tests, so
// a transform written inside a `View` can be checked by nothing but a person
// swiping at it. `ShellFlightTests` is what that buys — the scale a page is
// drawn at, the rectangle the crop keeps, the corner it travels through and
// where on the screen it lands are all one arithmetic expression each, and
// every one of them has been wrong at least once in a way that reads as the
// page vanishing rather than as a number being off.
//
// The VIEW half stayed in `apps/ios/FarCooler/ShellPageLayer.swift`: the order
// the clip, the scale and the shadow compose in, and what the page carries as
// it goes. That is about SwiftUI, and it is not a thing a test can hold.
//
// Internal, like everything else in this package that the phone compiles
// directly — see `ShellNavigation.swift`'s header. The Mac depends on AgentKit
// as a real module and cannot see a name from here.

/// The numbers this shell MOVES by, as opposed to the ones it decides by.
///
/// Deliberately not in `ShellMetrics`. That enum is the gesture's arithmetic —
/// thresholds a release is measured against, every one of them covered by
/// `swift test` — and a corner radius or the scale that says "card" has no
/// true answer a test could assert. These are looked at, not proved, and keeping the two
/// kinds of number apart is what stops somebody tuning a corner and moving a
/// commit threshold by accident.
enum ShellMotion {
    /// The iPhone's own screen corner, which is what a page that has lifted
    /// off the screen has to grow to look like.
    ///
    /// This is the app switcher's trick and it is most of why a shrunken app
    /// there reads as a DEVICE rather than as a picture: the card keeps the
    /// display's corner. There is no public API for the real radius, and the
    /// number only has to be close.
    static let screenCorner: CGFloat = 55

    /// One overview card's corner. Shared with `ShellOverview` so the page
    /// arrives at exactly the shape it is becoming, rather than at a second
    /// number that happens to match today.
    static let cardRadius: CGFloat = 16

    /// How far a page travels while it turns into a card.
    ///
    /// One number for BOTH directions, and that is the point rather than
    /// thrift. The two gestures that take a page off the display — lifting it
    /// past the last row, and crossing sideways to another workspace — are the
    /// same object doing the same thing, so a page that rounded its corners
    /// over 24 points going up and over a page width going sideways would be
    /// two different objects that happen to share a screen.
    ///
    /// Short, because the corner is not information: it is what says the thing
    /// under your finger has left the glass, and it should have said it by the
    /// time you have moved far enough to mean it.
    static let cardReveal: CGFloat = 24

    /// What a page is scaled to once it has become a card in the PLANE of the
    /// screen — a sideways crossing, where it is not receding, just detached.
    ///
    /// Four percent, which on a 393-point page is a 16-point gap between the
    /// card leaving and the card arriving. That gap is the whole reason the
    /// number is not 1: two full-bleed pages sliding edge to edge are one
    /// plane scrolling, and the same two with daylight between them are two
    /// cards. Small enough that the text inside stays crisp.
    static let crossingScale: CGFloat = 0.96

    /// How far the ground behind the cards is darkened while they are crossing.
    ///
    /// Without it the shrink is invisible: the page's ground and the ground
    /// behind it are the same colour, so a card that pulls in from the edges
    /// reveals more of exactly what it was already covering. The dim is what
    /// turns the gap into a gap.
    static let deskDim: CGFloat = 0.45

    /// How small a page gets while it is still IN YOUR HAND.
    ///
    /// Not the cell's size, and that distinction is the whole of it. The page
    /// used to shrink all the way to the tile's own width under the finger —
    /// on this display 168 into 402, which is 42% — so by the top of a lift
    /// you were holding a thumbnail and the release had nothing left to do but
    /// slide it. Two things were wrong with that. It is too much motion for a
    /// gesture you might abandon: 58% of the screen is spent before you have
    /// decided anything. And it spends the flight's own budget early, so
    /// letting go was a translation rather than an arrival.
    ///
    /// A held page comes off the glass and stays a PAGE. It is the release
    /// that turns it into a card, and the rest of the shrink is what carries
    /// that.
    static let heldScale: CGFloat = 0.82

    /// How much travel past the last row the shrink spends.
    ///
    /// **Not `ShellMetrics.overRun`, deliberately, and this is the other half
    /// of "it reaches its final size early and then just sits there."** The
    /// overrun is a THRESHOLD — 76 points past the last row is where a release
    /// stays in the overview — and it is the right distance for a decision and
    /// far too short for a motion. A real lift is three hundred points up a
    /// phone; a shrink normalized by 76 of them is finished in the first
    /// quarter of the gesture and holds still for the rest, which is a page
    /// that stopped answering the finger still moving it.
    ///
    /// Twice the overrun, so the page is still visibly shrinking through the
    /// part of the drag where a person is deciding, and reaches `heldScale`
    /// near the end of a lift somebody meant.
    ///
    /// A number in this enum rather than in `ShellMetrics`, and the line is
    /// the one that enum's header draws: nothing is measured against this and
    /// no release is decided by it. It is looked at.
    static let liftReach: CGFloat = ShellMetrics.overRun * 2

    /// The curve the held shrink runs on, from points of travel to 0…1.
    ///
    /// **Quadratic ease-in: gentle at the start, most of the shrink late.** A
    /// linear ramp was the specific complaint — the finger's travel is linear
    /// and a page that answers it linearly reads as a slider rather than as an
    /// object being picked up. Starting gently is also what the first points
    /// past the last row need: they are the ones a thumb overshoots a menu
    /// into, and a page that jumps backwards there turns a misjudged menu into
    /// a scare. The acceleration afterwards is the page committing to leaving.
    ///
    /// It cannot be a spring or a duration. This is not an animation; it is
    /// the position of a thing under a finger, evaluated fresh at every touch
    /// point, and a curve is the only kind of easing a value with no clock can
    /// have.
    static func lifted(_ travel: CGFloat) -> CGFloat {
        let t = min(1, max(0, travel / liftReach))
        return t * t
    }

    /// A page that has left the display casts one of these.
    ///
    /// The reason a lifted page needs one at all is that everything else about
    /// the lift is a scale, and a scale alone is ambiguous: a rectangle that
    /// is smaller than the screen is either above the screen or drawn on it.
    /// The shadow is what picks. Without it the page reads as pasted onto the
    /// grid — a picture in a grid of pictures — rather than as the one card
    /// that is in your hand, which is the whole sentence the gesture is
    /// saying.
    ///
    /// Wide and soft and well below the card, which is what a thing held a
    /// long way off a surface casts. A tight shadow reads as a sticker.
    static let liftShadowRadius: CGFloat = 26
    static let liftShadowY: CGFloat = 16
    static let liftShadowOpacity: CGFloat = 0.55

    /// The menu arriving, and leaving.
    ///
    /// A spring rather than the `interactiveSpring` the lift is written
    /// inside, because the column does not follow a finger: it is shut, then
    /// whole. Run on the tracking spring it appeared as if switched on, which
    /// is what the owner called too sudden — the tracking spring is tuned to
    /// be imperceptible, which is right for a thing that tracks and wrong for
    /// a thing that arrives.
    ///
    /// **Sprightly, and not a flash.** Both halves of that are numbers here.
    /// The response is long enough that the eye can follow the surface open
    /// rather than find it already open — a spring under about a third of a
    /// second is a step function with a spring's name on it — and the damping
    /// is low enough to overshoot slightly and settle back, which is what
    /// makes it read as sprung rather than as timed. The same pair runs the
    /// close, including the pop-shut past the last row: that one has to be a
    /// pop you can SEE, not a disappearance.
    /// 0.28, down from 0.42.
    ///
    /// 0.42 with light damping was chosen to answer "it flashes in", and
    /// overshot: the owner's word for it was motion sick. A menu is a small
    /// object travelling a short distance and it should be over before you
    /// have finished the gesture that asked for it. Still under-damped, so it
    /// arrives with life rather than stopping dead — the fix for a flash was
    /// never duration, it was the overshoot.
    static var menu: Animation { .spring(response: 0.28, dampingFraction: 0.7) }

    /// The same menu, for the changes no finger threw.
    ///
    /// **The same response and no overshoot**, so it is the same object moving
    /// at the same speed and only the ringing is gone. Three of the four ways
    /// the column changes are like this: it is TAPPED open, it furls after a
    /// row is chosen, and it pops shut past the last row while the thumb is
    /// still travelling the other way. Only the drag that opens it is going
    /// where the finger is going, and that one keeps `menu`.
    ///
    /// See `ShellRootView.syncMenu`, which is the single place the two are
    /// chosen between, and which carries the frames this was decided on.
    static var menuSettled: Animation { .spring(response: 0.28, dampingFraction: 1) }

    /// The elongated mark travelling between marks.
    ///
    /// Its own animation and not the release's, because what moves is a width
    /// inside a bar that is itself holding still — see `ShellRibbon.shown`.
    static var ribbon: Animation { .spring(response: 0.3, dampingFraction: 0.82) }
}

/// The flying page's geometry: where it is drawn, how big, how much of it, and
/// with what corner.
///
/// **Every function here is total and takes everything it needs.** They were
/// computed properties on `ShellRootView` reading eight pieces of `@State`
/// between them, which is the same arithmetic with no way to ask it a
/// question: "a page halfway through a flight into the third cell" is a
/// sentence a test can write here and could not write there.
///
/// Nothing in this enum knows about a gesture. `rise` is however far past the
/// last row the finger has gone — `ShellGesture.pageRise` is who answers that
/// — and `cropped` is however far through the flight the release's spring has
/// carried the page. Both arrive as numbers.
enum ShellFlight {
    /// How much of a card a page that has travelled this far off the display
    /// has become, 0…1. The same ramp whichever way it went.
    static func cardness(travel: CGFloat) -> CGFloat {
        min(1, max(0, travel / ShellMotion.cardReveal))
    }

    /// How far off the display the page is, 0…1, whichever half of the journey
    /// it is in — under a finger, or in the air.
    ///
    /// The shadow reads this rather than either half on its own. The rise
    /// covers the lift and goes to zero the instant `overview` is set, and
    /// `cropped` covers the flight and is zero for the whole lift; a shadow on
    /// only one of them would switch off at the exact moment of release, which
    /// is the one frame in the gesture where nothing at all should change.
    static func offGlass(rise: CGFloat, cropped: CGFloat) -> CGFloat {
        max(cardness(travel: rise), cropped)
    }

    /// How much smaller the page is drawn than the display, at this moment.
    ///
    /// Two shrinks in series, and they are separate because they belong to
    /// different halves of the gesture. The first is what a FINGER does: an
    /// eased ramp from the display down to `ShellMotion.heldScale`, over the
    /// run past the last row, with the page still a page. The second is what
    /// the RELEASE does, on `cropped`: the rest of the way down to the cell's
    /// own width.
    ///
    /// The second term reads `cropped` rather than "is the overview open" for
    /// the reason `cropped` exists at all — the overview flag and the flight
    /// count both change outside the animation that carries the page, so a
    /// scale written against either of them is a scale that jumps and then
    /// travels.
    static func scale(page: CGRect, tile: CGRect?, rise: CGFloat, cropped: CGFloat) -> CGFloat {
        guard page.width > 0 else { return 1 }
        let held = 1 - (1 - ShellMotion.heldScale) * ShellMotion.lifted(rise)
        guard let tile, tile.width > 0 else { return held }
        return held + (tile.width / page.width - held) * cropped
    }

    /// How much of the page is drawn, in the page's own coordinates.
    ///
    /// **On the same ramp as the width, so the shape is finished before you
    /// let go.** This used to crop only at the destination — the page stayed a
    /// whole screen all the way up and collapsed to a card's height during the
    /// flight — and that collapse is the other half of what the owner called
    /// vanishing into the hole. A page shrunk to a cell's WIDTH is still
    /// nearly three cells TALL, so what you were holding at the top of the
    /// lift was a sliver lying across three rows of the grid, and letting go
    /// of it made it fall in on itself as it travelled. Two motions, one of
    /// them unasked for.
    ///
    /// The price is honest and small: the top of the page is eaten as the
    /// flight runs, which is the end of it you had already let go of, and the
    /// bottom — the live end of a terminal, the part the card's own tail is
    /// about — is what stays under your thumb the whole way.
    static func height(page: CGRect, tile: CGRect?, cropped: CGFloat) -> CGFloat {
        guard let tile, tile.width > 0, page.width > 0 else { return page.height }
        // The cell's height, said in page coordinates — the clip happens
        // before the scale, so this is what draws as `tile.height` once the
        // page is tile-sized.
        let card = tile.height * page.width / tile.width
        // Cropped on `cropped`, which means: not at all while the finger is
        // down, and all the way across the flight.
        //
        // The first half is the property the owner asked for. A page being
        // minimized is still a page the whole way up — the app switcher
        // shrinks an app, it does not reshape it while your thumb is on it —
        // and turning into a card is what happens AFTER you let go, because
        // becoming a card is the same event as landing in the grid.
        //
        // The second half is a bug this shell wrote and then had to fix. It
        // read `isFlying ? card : page.height`, which is a STEP, and the flag
        // it stepped on is set outside the animation that carries the page —
        // so the page's height jumped at the instant of release and only then
        // travelled, and the same thing ran backwards on the way out. An
        // interpolatable value is the whole of the fix.
        return page.height + (card - page.height) * cropped
    }

    /// A page has the display's corners and a tile has a card's, so the corner
    /// travels too.
    ///
    /// Two things in one number, in order. First the display's own corner, over
    /// the short travel `cardReveal` measures: a page at rest is clipped by the
    /// display itself, so this is invisible until the page has actually left
    /// it — and it is drawn rather than assumed, because a page that keeps the
    /// display's corner even when the display is not the thing clipping it is
    /// most of why an app switcher card reads as a device. Then the card's own
    /// radius, over the run into the overview, EXPRESSED IN PAGE COORDINATES —
    /// the clip happens before the scale, so `cardRadius / scale` is what draws
    /// as `cardRadius` once the page is tile-sized.
    ///
    /// The second half finishing exactly when the shrink does is most of what
    /// makes the release simple: by the time you let go the page is already
    /// the cell's width and the cell's corner, and all that is left is to go
    /// there and lose the height a card does not have.
    ///
    /// Written as the radius you SEE and divided back into page coordinates at
    /// the end, rather than kept in page coordinates throughout. The clip
    /// happens before the scale, so a fixed number here draws smaller as the
    /// page shrinks — which is why a page held off the glass used to lose the
    /// display's corner exactly as it became the thing that most needed it.
    /// What the app switcher does is keep the DISPLAY's corner while the card
    /// is a device and take the card's once it is a card, and both of those
    /// are facts about the screen.
    static func radius(scale: CGFloat, rise: CGFloat, cropped: CGFloat) -> CGFloat {
        guard scale > 0 else { return 0 }
        let corner = ShellMotion.screenCorner * cardness(travel: rise)
        return (corner + (ShellMotion.cardRadius - corner) * cropped) / scale
    }

    /// Where the page sits, and the two phases it sits there in.
    ///
    /// **While the finger is down the page does only what the finger does.**
    /// It rises and it shrinks, and its BOTTOM EDGE is pinned to the travel
    /// past the last row — one point of page for one point of drag, so the
    /// thing under your thumb is the thing your thumb is moving. Written
    /// against the top-leading anchor: at scale `s` the page's bottom lands at
    /// `height * s + offset`, so pinning it to `height - rise` is the
    /// arithmetic below and nothing more.
    ///
    /// **And sideways too.** The horizontal term is the shrunken page's own
    /// centre PLUS `carryX`, which is the finger, one point per point. It used
    /// to be the centre alone, which was a correct fix for the wrong amount of
    /// the problem: the page must not AIM at its destination cell mid-drag —
    /// that is the curve that goes up and then suddenly sideways — but a card
    /// that ignores your thumb sideways is a card on rails, and no card in the
    /// app switcher or in Safari's tab switcher is on rails. The destination
    /// still only becomes a destination when you let go; what changed is that
    /// until then the card is where your thumb put it, in both directions.
    ///
    /// It does NOT drift toward the cell it is going to. That was the shape of
    /// the curve the owner called out — up, then suddenly sideways — and it
    /// comes from treating the destination as a target while the gesture is
    /// still running. The cell only becomes a target when you let go, and then
    /// the page flies to it from wherever it is, as one spring. That is what
    /// the app switcher does: the card follows your thumb, and settles into
    /// the grid only after you let go.
    ///
    /// Which is why `landing` is a plain answer rather than a blend. It is the
    /// state a release puts the shell into, so the spring that runs it
    /// interpolates from whatever the page was last drawn at — including its
    /// velocity — to the cell, with nothing in between for a mid-drag
    /// destination to bend.
    static func offset(
        page: CGRect, tile: CGRect?, landing: Bool, scale: CGFloat, rise: CGFloat,
        anchorX: CGFloat, carryX: CGFloat
    ) -> CGSize {
        guard page.width > 0 else { return .zero }
        if landing, let tile {
            // Written against the page's BOTTOM edge, because that is the edge
            // the clip keeps: the drawn rectangle starts `page.height * scale`
            // above where an uncropped page's bottom would land, so putting
            // that bottom on the cell's bottom is what puts the whole drawn
            // rectangle exactly on the cell.
            return CGSize(
                width: tile.minX - page.minX,
                height: tile.maxY - page.minY - page.height * scale)
        }
        // Anchored at the point the drag went down, not at the middle of the
        // display. At scale `s` a page point `x` is drawn at `x * s + offset`,
        // so holding the anchor still is `anchor * (1 - s)` and nothing more —
        // and holding it still is the whole requirement: the pixels under a
        // thumb that has not moved sideways must not move sideways.
        return CGSize(
            width: anchorX * (1 - scale) + carryX,
            height: page.height * (1 - scale) - rise)
    }

    /// Where a page sits while a FINGER is taking it back out of the grid.
    ///
    /// **The straight line between `offset`'s two branches, walked by a
    /// thumb.** Those branches are the two ends of this journey already — the
    /// cell a card is drawn in, and the display a page fills — and the flight
    /// that runs between them on release is a spring interpolating one into
    /// the other. A pull-down is that same interpolation with the clock taken
    /// off it, so this is the same arithmetic and not a second opinion about
    /// where a returning page goes.
    ///
    /// `progress` 0 is the cell and 1 is the display. At 0 it is byte for byte
    /// `offset(landing: true)` and at 1 it is the display's own origin, which
    /// is the property that lets a pull begin and end without a step: the
    /// first frame of a tracked pull draws the page exactly where the grid was
    /// already drawing its card, and the last frame draws it exactly where a
    /// finished flight leaves it.
    ///
    /// Only the PLACE. How big the page is drawn is `scale`'s job and how much
    /// of it is drawn is `height`'s, and both of those already read `cropped`
    /// — which the same thumb is driving. Three functions of one progress, and
    /// no fourth thing to keep in step.
    ///
    /// The home end is anchored at the middle of the display rather than under
    /// the finger, and that matches the release flight's own rule: a page
    /// growing back out of a cell has no grab point on it — the finger is on
    /// the grid, not on the page — so growing about its own centre is the only
    /// unbiased answer. See `ShellPageLayer.shrinkAnchorX`, which says the
    /// same thing about the same journey run by a spring.
    static func returning(
        page: CGRect, tile: CGRect?, scale: CGFloat, progress: CGFloat
    ) -> CGSize {
        guard page.width > 0 else { return .zero }
        let home = offset(
            page: page, tile: tile, landing: false, scale: scale, rise: 0,
            anchorX: page.width / 2, carryX: 0)
        guard let tile else { return home }
        let cell = offset(
            page: page, tile: tile, landing: true, scale: scale, rise: 0, anchorX: 0,
            carryX: 0)
        let along = min(1, max(0, progress))
        return CGSize(
            width: cell.width + (home.width - cell.width) * along,
            height: cell.height + (home.height - cell.height) * along)
    }
}
