import SwiftUI

// The page, as one moving object: the shape it is clipped to, the layer that
// carries it, and the transforms that put it where a finger or a spring says.
//
// Split out of `ShellRootView` because that file had become the whole shell —
// the state, the finger, the motion and the layering in one type — and these
// are the part that is about a PICTURE. What is here is only what SwiftUI
// itself makes true: the order the clip, the scale and the shadow have to
// compose in.
//
// The arithmetic is NOT here. Every number below comes out of
// `AgentKit/ShellFlight.swift`, which is where `swift test` can reach it —
// same division as `ShellNavigation.swift` makes for the thresholds, and for
// the same reason: a transform written inside a `View` can be checked by
// nothing but a person swiping at it, and each of these has been wrong at
// least once in a way that reads as the page vanishing.
//
// The shell is over one worktree, so a lifted page has nowhere to fly to: it
// is held under the finger, shrunk and shadowed, and put back on release.

/// The shape the lifted page is clipped to: a rounded rectangle over the
/// BOTTOM `height` of whatever it is handed.
///
/// `Animatable` on purpose and not incidentally. The corner has to travel
/// with the spring that carries the page — a clip that jumped to its final
/// shape on the first frame would be the page arriving before it left — and a
/// shape only interpolates if it says how.
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

    /// Everything that is the worktree you are in, as one moving object.
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
            .clipShape(ShellFlightShape(height: flightHeight, radius: flightRadius))
            // Anchored top-leading so the scale and the offset compose
            // predictably: with a center anchor the offset would have to carry
            // half the shrink as well, which is the arithmetic that makes this
            // kind of thing land a few points off.
            .scaleEffect(flightScale, anchor: .topLeading)
            // AFTER the scale, so the radius is in screen points rather than
            // in the page's own — a shadow drawn before the scale would shrink
            // with the card and get tighter exactly as the card gets further
            // away, which is backwards.
            .shadow(  // style-exempt: the lift shadow of a page in flight, whose radius is in screen points
                color: .black.opacity(ShellMotion.liftShadowOpacity * offGlass),  // style-exempt: the lift shadow of a page in flight
                radius: ShellMotion.liftShadowRadius, x: 0,
                y: ShellMotion.liftShadowY * offGlass)
            .offset(x: flightOffset.width, y: flightOffset.height)
    }

    /// How far past the last row the finger has gone, which is the only part
    /// of the lift the page answers at all.
    ///
    /// `pageAbove` and not `lift`: the page's own rise is a question about
    /// where the finger IS relative to the column's top edge, not about how
    /// far it has travelled since touch-down — see `pageAbove`'s header,
    /// and `ShellGesture.pageRise`'s, for the drag that starts low in the
    /// bar this used to get wrong.
    private var pageRise: CGFloat {
        ShellGesture.pageRise(up: pageAbove, tabCount: tabCount)
    }

    /// How far off the display the page is, 0…1. See `ShellFlight.offGlass`.
    private var offGlass: CGFloat {
        ShellFlight.offGlass(rise: pageRise, cropped: 0)
    }

    /// How much smaller the page is drawn than the display, at this moment.
    /// See `ShellFlight.scale`, which is where the shrink is written out.
    private var flightScale: CGFloat {
        ShellFlight.scale(page: pageFrame, tile: nil, rise: pageRise, cropped: 0)
    }

    /// The point of the page the shrink is anchored at, in page coordinates.
    ///
    /// Where the finger that lifted it went down, and the middle of the
    /// display until one has.
    private var shrinkAnchorX: CGFloat {
        liftOrigin ?? pageFrame.width / 2
    }

    /// Where the page sits, under the finger. See `ShellFlight.offset`.
    private var flightOffset: CGSize {
        ShellFlight.offset(
            page: pageFrame, tile: nil, landing: false, scale: flightScale,
            rise: pageRise, anchorX: shrinkAnchorX, carryX: carryX)
    }

    /// A page has the display's corners, so the corner travels with the
    /// shrink. See `ShellFlight.radius`.
    private var flightRadius: CGFloat {
        ShellFlight.radius(scale: flightScale, rise: pageRise, cropped: 0)
    }

    /// How much of the page is drawn, in the page's own coordinates: all of
    /// it. See `ShellFlight.height`.
    private var flightHeight: CGFloat {
        ShellFlight.height(page: pageFrame, tile: nil, cropped: 0)
    }
}
