import AgentKit
import SwiftUI

/// The detail area's shape, in one place.
///
/// The original problem: an `NSView` filling the detail pane is a rectangle, and
/// the window's bottom-right corner is not. Clipping the terminal itself with two
/// rounded corners and two square ones sort of worked, and looked like what it
/// was — a rectangle pretending, meeting a sidebar that curved away from it.
///
/// So the terminal stops touching the window. The CANVAS meets the window and
/// takes its corner; the terminal is a fully rounded card floating on it. Nothing
/// has to fake anything, one pane looks like a one-pane layout, and four panes
/// look like four of the same thing — which they are.
///
/// It costs about a column each side. Worth it: the alternative is a seam at the
/// one corner of the app the eye is drawn to.
enum Pane {
    /// Inset around the cards.
    ///
    /// Measured, not chosen. A window corner of radius R cuts furthest into the
    /// content along the diagonal, by R(1 − 1/√2) ≈ 0.29R in each axis. This
    /// window's corner measures about 25pt — macOS 26 rounds windows far more
    /// than earlier releases did — so the curve reaches roughly 7.3pt inside the
    /// corner, and a card inset by less than that gets bitten by it.
    ///
    /// 6pt was less than that, which is exactly what "touching the corner"
    /// looked like: the card's own arc running into the window's, two mismatched
    /// curves a couple of points apart. That was macOS 26's corner. macOS 27's
    /// is about 16, so its gutter is the corner minus the card's radius, 6, and
    /// the card shares the corner's center (`Gutter.window`).
    static var inset: CGFloat { Gutter.window }

    /// The cards' radius: `Radius.medium`, the step every card is drawn in.
    static let radius: CGFloat = Radius.medium
}

extension View {
    /// The backdrop the panes float on.
    ///
    /// Deliberately NOT clipped to a corner radius of its own. It used to be, with
    /// a hardcoded 10 — which is where the bad corner came from: this window's is
    /// about 25, so a 10pt clip was nearly square by comparison and the window cut
    /// through it. Guessing a number that has changed twice across macOS releases
    /// is the wrong shape of fix.
    ///
    /// The window already clips its own content to its own shape, whatever that
    /// shape currently is. All this has to do is keep the cards far enough from
    /// the corner that the curve never reaches them — which is `Pane.inset`'s job.
    func paneCanvas() -> some View {
        padding(Pane.inset)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            // Nothing painted (ov-220): the window's frosted plane shows in the
            // gutter around the cards. It used to be `canvas`, the window's own
            // color blended 5% toward the terminal theme; the theme's hue now
            // tints the cards only, and the wallpaper supplies the plane's.
    }

    /// One terminal, as a card on the plane.
    ///
    /// Opaque paper in the theme-tinted document color, in `Radius.medium`
    /// corners, with no stroke (ov-221): the card against the frosted plane is
    /// its own edge, and Increase Contrast draws a 1 px separator back around it
    /// (`Surface.content`). The pane's header is drawn on the card's own color,
    /// not a wash of its own.
    ///
    /// Three versions of a focus border were tried here and every one of them
    /// was the loudest thing on screen: a blue ring (fine over a VT grid, awful
    /// around a chat), a drop shadow (which a full-window pane smeared across
    /// the chrome above it), and a thicker grey edge (just as heavy as the blue
    /// one, minus the color). Focus is not drawn on the card at all: the pane's
    /// header says it, with the number and title in primary ink and the title
    /// in semibold, against secondary for every other pane.
    func paneCard() -> some View {
        clipShape(.card).surface(.content, in: .card, fill: WorkspaceStyle.document)
    }
}
