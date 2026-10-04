import AgentKit
import CoreGraphics
import Testing

@testable import Far_Cooler

/// The gutter around the cards is concentric with the window's corner (ov-220).
///
/// The corner itself can't be read from here: the window server draws the
/// mask, and capturing a window from another process needs Screen Recording.
/// So the corners are pinned as measured (ov-216 design, §4.1: about 16 pt on
/// macOS 27, about 25 on macOS 26), and the gutter is pinned to the corner
/// minus the card's radius, where 26's would be too wide to sit clear of the
/// curve and keeps the 10 it always had.
struct WindowCornerTests {
    @Test("On macOS 27 the gutter is the window's corner minus a card's radius")
    func concentricOn27() {
        let corner: CGFloat = 16
        #expect(Gutter.window(osMajor: 27) == corner - Radius.medium)
        #expect(Gutter.window(osMajor: 28) == corner - Radius.medium)
    }

    @Test("On macOS 26 the gutter keeps its 10 points, clear of the 25 point corner")
    func clearOn26() {
        #expect(Gutter.window(osMajor: 26) == 10)
        // A 25 pt corner reaches 25 * (1 - 1/sqrt 2), about 7.3 pt, into a
        // card's corner: the 10 clears it, a concentric 15 would be too much.
        #expect(Gutter.window(osMajor: 26) > 25 * (1 - 1 / 2.0.squareRoot()))
    }

    @MainActor
    @Test("The cards' inset and the columns' chrome both follow the gutter")
    func theInsetFollows() {
        #expect(Pane.inset == Gutter.window)
        // Two gutters and the terminal's own 10 pt each side.
        #expect(WorkspaceColumns.chrome == 2 * Gutter.window + 20)
    }
}
