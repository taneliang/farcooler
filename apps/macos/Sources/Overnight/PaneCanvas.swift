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
    static let inset: CGFloat = 6

    /// The cards' radius.
    ///
    /// Near-concentric with the window's corner given the inset, which is what
    /// makes an inset card look placed rather than pasted on.
    static let radius: CGFloat = 6

    /// The window's own bottom corner, as macOS draws it.
    static let windowRadius: CGFloat = 10
}

extension View {
    /// The backdrop the panes float on: inset, and shaped to the window.
    func paneCanvas() -> some View {
        padding(Pane.inset)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(nsColor: .underPageBackgroundColor))
            // Only the trailing corner is rounded: the leading edge meets the
            // sidebar and the top meets the title bar, and rounding either would
            // open a gap rather than close one.
            .clipShape(
                UnevenRoundedRectangle(
                    bottomLeadingRadius: 0,
                    bottomTrailingRadius: Pane.windowRadius,
                    topTrailingRadius: 0))
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
