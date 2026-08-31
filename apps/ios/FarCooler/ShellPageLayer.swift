import SwiftUI

// The page, as one moving object: the shape it is clipped to, the layer that
// carries it, and the transforms that put it where a finger or a spring says.
//
// Split out of `ShellRootView` because that file had become the whole shell —
// the state, the finger, the motion, the layering and the overview's lifecycle
// in one type — and these are the part that is about a PICTURE. What is here
// is only what SwiftUI itself makes true: the order the clip, the scale and
// the shadow have to compose in, and the fact that the card the page becomes
// has to be drawn ON the page rather than arriving separately.
//
// The arithmetic is NOT here. Every number below comes out of
// `AgentKit/ShellFlight.swift`, which is where `swift test` can reach it —
// same division as `ShellNavigation.swift` makes for the thresholds, and for
// the same reason: a transform written inside a `View` can be checked by
// nothing but a person swiping at it, and each of these has been wrong at
// least once in a way that reads as the page vanishing.

/// The shape the flying page is clipped to: a rounded rectangle over the
/// BOTTOM `height` of whatever it is handed.
///
/// It exists because a page and a card are not the same shape and no scale can
/// make them one. The page is the display — 402 by 874 here — and a card is
/// 168 by 132, so a page shrunk until it is a card's WIDTH is still nearly
/// three times a card's height. Scaled to the height instead it is too narrow;
/// scaled to both it is a squashed picture of a terminal. What is left is what
/// the system itself does when a snapshot has to land in a frame that is not
/// its aspect: keep the picture honest and CROP it, so the page arrives at
/// exactly the rectangle the grid is holding open and the handover is a change
/// of content rather than a change of shape.
///
/// The BOTTOM of the page and not the top, for two reasons that agree. It is
/// the half your thumb was holding all the way up — the crop should take away
/// the end of the page you had already let go of — and it is where a terminal
/// keeps its most recent lines, which is exactly what the card is about to
/// show in its tail. Cropped the other way the page would land showing the top
/// of a screen and then cross-fade to the bottom of it.
///
/// `Animatable` on purpose and not incidentally. The crop and the corner both
/// have to travel with the spring that carries the page — a clip that jumped
/// to the card's shape on the first frame of the flight would be the page
/// arriving before it left — and a shape only interpolates if it says how.
private struct ShellFlightShape: Shape {
    /// How much of the page is drawn, in the page's own coordinates.
    var height: CGFloat
    var radius: CGFloat

    var animatableData: AnimatablePair<CGFloat, CGFloat> {
        get { AnimatablePair(height, radius) }
        set {
            height = newValue.first
            radius = newValue.second
        }
    }

    func path(in rect: CGRect) -> Path {
        let drawn = min(rect.height, max(0, height))
        return Path(
            roundedRect: CGRect(
                x: rect.minX, y: rect.maxY - drawn, width: rect.width, height: drawn),
            cornerRadius: max(0, radius), style: .continuous)
    }
}

extension ShellRootView {
    // MARK: - The page, and its flight

    /// Everything that is the workspace you are in, as one moving object.
    ///
    /// Order matters here in a way that is easy to get wrong twice over. The
    /// clip comes BEFORE the scale, because `scaleEffect` does not change a
    /// view's layout size: a `clipShape` after it is sized to the full page
    /// and clips nothing at all, which is how a rounded corner that was
    /// definitely being applied managed to be invisible. Clipped first, the
    /// radius is drawn at page scale and shrinks with everything else, so the
    /// number below is in the page's own coordinates and the number you SEE is
    /// that times the scale.
    ///
    /// A transform, deliberately, and not `matchedGeometryEffect`: that
    /// animates the FRAME, so the pane would re-lay-out at every size on the
    /// way down — a terminal reflowing to 168 points wide, forty times a
    /// second. The app switcher scales a rigid picture of the app, and
    /// `scaleEffect` plus `offset` is that picture. Nothing inside re-flows; it
    /// just gets smaller.
    func pageLayer(page: CGFloat, safeArea: EdgeInsets) -> some View {
        paneTrack(page: page, safeArea: safeArea)
            // The page carries the CARD it is turning into, and that is the
            // whole of the landing.
            //
            // See `cardFace`. Drawn inside the clip and inside the scale, so
            // it travels as part of the page rather than as a second thing
            // arriving at the same address.
            .overlay(alignment: .bottom) { cardFace }
            .clipShape(ShellFlightShape(height: flightHeight, radius: flightRadius))
            // Anchored top-leading so the scale and the offset compose
            // predictably: with a centre anchor the offset would have to carry
            // half the shrink as well, which is the arithmetic that makes this
            // kind of thing land a few points off.
            .scaleEffect(flightScale, anchor: .topLeading)
            // AFTER the scale, so the radius is in screen points rather than
            // in the page's own — a shadow drawn before the scale would shrink
            // with the card and get tighter exactly as the card gets further
            // away, which is backwards.
            .shadow(
                color: .black.opacity(ShellMotion.liftShadowOpacity * offGlass),
                radius: ShellMotion.liftShadowRadius, x: 0,
                y: ShellMotion.liftShadowY * offGlass)
            .offset(x: flightOffset.width, y: flightOffset.height)
            // Drawn all the way in, and only handed over once it has landed.
            //
            // The page is the current workspace's card for as long as it is in
            // the air — the grid is holding an empty cell open for it — so it
            // cannot fade out on the way: a page that dissolved as it arrived
            // would land in a hole and leave one. What it does instead is
            // change places with the card, at rest, in the same rectangle, on
            // `handover`, over a card that is already opaque. See `land()`.
            .opacity(pageAlpha)
            .allowsHitTesting(!overview)
    }

    /// How far past the last row the finger has gone, which is the only part
    /// of the lift the page answers at all.
    private var pageRise: CGFloat {
        overview ? 0 : ShellGesture.pageRise(up: lift, tabCount: tabCount)
    }

    /// How far off the display the page is, 0…1, whichever half of the journey
    /// it is in — under a finger, or in the air. See `ShellFlight.offGlass`.
    private var offGlass: CGFloat {
        ShellFlight.offGlass(rise: pageRise, cropped: cropped)
    }

    /// The card the page is turning into, drawn on the page itself.
    ///
    /// **The landing is not a handover between two objects; it is one object
    /// arriving as what it becomes.** What used to land in the cell was the
    /// bottom of a terminal — a grey rectangle — and the finished card faded
    /// in over it once everything had stopped. Putting an opaque card
    /// underneath first and dissolving the page over it removed the see-through
    /// dip but not the sequence: the card still resolved after the arrival,
    /// and two steps is what the owner keeps seeing.
    ///
    /// So the flying page carries a real `ShellCardFace` — the same view the
    /// grid draws, not one that looks like it — and it fades in on `cropped`,
    /// which is the flight's own progress. By the time the page is on the cell
    /// it IS the card: name, tail, ribbon and subtitle, at the cell's size, in
    /// the cell's place. The handover in `land()` is then a cross-fade between
    /// two identical drawings of one workspace in one rectangle, which is a
    /// cross-fade nobody can see.
    ///
    /// Laid out at the card's own size and SCALED, never re-laid-out. The
    /// drawn region shrinks from a whole page to a card over the flight, and a
    /// card asked to fill it at every intermediate height would spring its
    /// tail away from its ribbon on every frame. Bottom-anchored inside that
    /// region for the same reason the crop keeps the page's bottom: the edge
    /// the flight is written against is the bottom edge, so the card sits
    /// perfectly still against it while the band above it closes.
    ///
    /// The band is filled rather than left open — the card's own ground over
    /// an opaque one — because a partly-faded card over a page that is being
    /// cropped away would show the grid through the gap, which is the dip
    /// again wearing a different hat.
    @ViewBuilder
    private var cardFace: some View {
        // Mounted from the moment the grid has measured a cell, and NOT gated
        // on `cropped > 0`. A view inserted into the tree at the start of an
        // animation has no previous opacity to interpolate from, so gating it
        // would make the card pop in whole on the first frame of the flight —
        // which is the exact failure this exists to remove, moved to the other
        // end of the journey.
        if let tile, tile.width > 0, tile.height > 0, pageFrame.width > 0,
            let workspace = currentWorkspace
        {
            let magnify = pageFrame.width / tile.width
            // The card at the size the crop is currently drawing, said in the
            // card's own coordinates — the whole thing is scaled back up by
            // `magnify` below, so a height of `flightHeight / magnify` draws
            // as exactly `flightHeight`, which is the rectangle the clip
            // keeps. The corner travels the same way and for the same reason.
            ShellCardFace(
                workspace: workspace, isCurrent: true,
                height: flightHeight / magnify, radius: flightRadius / magnify,
                opaqueGround: true)
                .scaleEffect(magnify, anchor: .bottom)
                .opacity(cropped)
                // The page underneath is what answers a finger. This is a
                // picture of where the page is going.
                .allowsHitTesting(false)
        }
    }

    /// How much smaller the page is drawn than the display, at this moment.
    /// See `ShellFlight.scale`, which is where the two shrinks are written out.
    private var flightScale: CGFloat {
        ShellFlight.scale(page: pageFrame, tile: tile, rise: pageRise, cropped: cropped)
    }

    /// The point of the page the shrink is anchored at, in page coordinates.
    ///
    /// The middle of the display until a finger says otherwise, which is what
    /// the reverse journey wants: a page growing back out of a cell has no
    /// finger on it, and growing about its own centre is the only unbiased
    /// answer.
    private var shrinkAnchorX: CGFloat {
        liftOrigin ?? pageFrame.width / 2
    }

    /// Where the page sits: under the finger, or on its way to the cell.
    /// See `ShellFlight.offset`, which is where the two phases are written out.
    ///
    /// `overview` is what says which phase this is. It is the state a release
    /// puts the shell into, so the spring that runs it interpolates from
    /// whatever the page was last drawn at — including its velocity — to the
    /// cell, with nothing in between for a mid-drag destination to bend.
    private var flightOffset: CGSize {
        ShellFlight.offset(
            page: pageFrame, tile: tile, landing: overview, scale: flightScale,
            rise: pageRise, anchorX: shrinkAnchorX, carryX: carryX)
    }

    /// A page has the display's corners and a tile has a card's, so the corner
    /// travels too. See `ShellFlight.radius`.
    private var flightRadius: CGFloat {
        ShellFlight.radius(scale: flightScale, rise: pageRise, cropped: cropped)
    }

    /// How much of the page is drawn, in the page's own coordinates — a whole
    /// screen while the finger is down, a card's rectangle by the end of the
    /// flight. See `ShellFlight.height`.
    private var flightHeight: CGFloat {
        ShellFlight.height(page: pageFrame, tile: tile, cropped: cropped)
    }

}
