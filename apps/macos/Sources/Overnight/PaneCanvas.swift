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
    /// curves a couple of points apart.
    static let inset: CGFloat = 10

    /// The cards' radius.
    ///
    /// Not derived from the window's, and deliberately: once a card is clear of
    /// the corner curve the two arcs are far enough apart that the eye stops
    /// relating them, and a concentric radius at this inset would be nearly
    /// square. 10 matches the platform's own language for an inset surface.
    static let radius: CGFloat = 10
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
            // `underPageBackgroundColor` is deliberately dark in BOTH appearances
            // — it is the colour behind a document page — which reads well against
            // a dark terminal and far too heavy in light mode. The window's own
            // background follows the appearance, which is what a backdrop should
            // do.
            .background(Color(nsColor: .windowBackgroundColor))
    }

    /// One terminal, as a card on the canvas.
    func paneCard(focused: Bool = false) -> some View {
        clipShape(RoundedRectangle(cornerRadius: Pane.radius))
            .overlay(
                RoundedRectangle(cornerRadius: Pane.radius)
                    .strokeBorder(
                        focused ? Color.accentColor : Color.primary.opacity(0.10),
                        lineWidth: focused ? 2 : 1)
            )
    }
}
