import CoreGraphics
import Foundation

/// A grid, in cells.
///
/// Its own type rather than a pair of `Int`s so that "the pane is 86 by 52" can
/// be compared, stored and passed without the two numbers ever swapping places.
struct PaneGrid: Equatable, Hashable {
    var columns: Int
    var rows: Int
}

/// The arithmetic between tmux's cells and the view's points.
///
/// Lifted out of `TileView` because it is pure, because it is the part that was
/// wrong, and because being wrong here is invisible: every number it produces is
/// plausible, and the symptom arrives later and somewhere else — as characters
/// left on screen in cells tmux does not believe it owns.
///
/// The direction of the whole thing matters more than any one line of it. The
/// app asks tmux for a WINDOW grid, once, from its own pixels; tmux answers with
/// a rectangle per pane; and from then on the panes' sizes are tmux's, not ours.
/// A client that recomputed a pane's grid from that pane's pixels would be
/// answering a question tmux has already answered — and answering it differently,
/// because the two round trips do not agree.
enum TileGeometry {
    /// The window grid to ask tmux for, in cells.
    ///
    /// Deliberately conservative. Every pane spends some of its rectangle on
    /// chrome — a header naming it, and the terminal's own insets — so a viewport
    /// computed as a flat width÷cell would have tmux hand each pane more columns
    /// than the renderer can draw, and every line long enough to reach the edge
    /// would wrap a second time. Subtracting the chrome, once per pane in each
    /// axis, errs the other way: a column or two of empty space at a pane's edge,
    /// which nobody notices.
    ///
    /// `across` and `down` are how many panes deep the layout is in each axis,
    /// which is what decides how many times the chrome is paid for.
    ///
    /// This used to carry a line saying it applied "the same floors the renderer
    /// applies to its own bounds, so the two land on the same grid rather than a
    /// cell apart." That was the bug, written down as a reassurance. The floors
    /// are the same; the numbers they are applied to are not, because this one
    /// measures the WINDOW and the renderer measured a PANE. In a 1400×900
    /// window laid out as an L, the full-height pane came out 88×59 measured
    /// against the 88×54 tmux had actually split — five rows of terminal that
    /// nothing would ever paint again.
    ///
    /// Main-actor only for one reason: it reads the pane header's height, which
    /// is part of the style and so lives where the views do. The rest of this
    /// type is arithmetic and stays free of that.
    @MainActor
    static func viewport(
        fitting size: CGSize, across: Int, down: Int, cell: CGSize
    ) -> PaneGrid? {
        guard cell.width > 0, cell.height > 0 else { return nil }

        let chromeWidth = TerminalMetrics.padding.left + TerminalMetrics.padding.right
        let chromeHeight =
            TerminalMetrics.padding.top + TerminalMetrics.padding.bottom
            + WorkspaceStyle.paneHeaderHeight

        let usableWidth = size.width - CGFloat(across) * chromeWidth
        let usableHeight = size.height - CGFloat(down) * chromeHeight
        guard usableWidth > 0, usableHeight > 0 else { return nil }

        return PaneGrid(
            columns: max(20, Int(usableWidth / cell.width)),
            rows: max(5, Int(usableHeight / cell.height)))
    }

    /// How many cells fit in a terminal view of this size.
    ///
    /// What a pane would LIKE to be, never what it is. In the single-terminal
    /// fallback that wish is granted, because there is nothing else asking; in a
    /// layout it is not, because tmux has already decided and this number is a
    /// cell or three away from that decision.
    ///
    /// The floors are the reason it can never be a way back to the pane's grid:
    /// a rectangle that is 51.7 cells tall measures 51, and so does one that is
    /// 51.0 cells tall. The information is gone.
    static func fitting(_ size: CGSize, cell: CGSize) -> PaneGrid? {
        guard cell.width > 0, cell.height > 0 else { return nil }
        let usableWidth = size.width - TerminalMetrics.padding.left - TerminalMetrics.padding.right
        let usableHeight = size.height - TerminalMetrics.padding.top - TerminalMetrics.padding.bottom
        guard usableWidth > 0, usableHeight > 0 else { return nil }
        return PaneGrid(
            columns: max(20, Int(usableWidth / cell.width)),
            rows: max(5, Int(usableHeight / cell.height)))
    }

    /// A pane's cell rectangle, in points.
    ///
    /// The gap between cards is not padding the view adds: it is the one cell
    /// tmux leaves between panes for a divider, scaled along with everything else.
    /// So the cards separate by exactly as much as tmux thinks they do, and a
    /// layout with no divider — one pane, or a zoomed one — fills the view edge to
    /// edge without a special case.
    ///
    /// Note what this does NOT do: round to a whole number of cells. It cannot,
    /// and it must not try. The panes have to tile the view exactly, and cells do
    /// not divide points evenly, so each pane's rectangle carries a fraction of a
    /// cell of slack. That slack is why measuring a pane's pixels is not a way to
    /// recover the pane's grid.
    static func frame(of rect: PaneRect, in window: PaneGrid, size: CGSize) -> CGRect {
        guard window.columns > 0, window.rows > 0 else { return CGRect(origin: .zero, size: size) }
        let x = CGFloat(rect.left) / CGFloat(window.columns) * size.width
        let y = CGFloat(rect.top) / CGFloat(window.rows) * size.height
        let width = CGFloat(rect.columns) / CGFloat(window.columns) * size.width
        let height = CGFloat(rect.rows) / CGFloat(window.rows) * size.height
        return CGRect(x: x, y: y, width: max(width, 1), height: max(height, 1))
    }

    /// How many panes deep a layout is, across and down.
    ///
    /// Taken from the rectangles rather than from the pane count: three panes
    /// side by side and three stacked spend chrome in different directions, and
    /// the count cannot tell them apart.
    static func depth(of panes: [PaneRect]) -> (across: Int, down: Int) {
        (
            across: max(1, Set(panes.map(\.left)).count),
            down: max(1, Set(panes.map(\.top)).count)
        )
    }
}
