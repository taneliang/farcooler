import AppKit
import Testing

@testable import Far_Cooler

/// Who decides how big a pane is.
///
/// The answer is tmux, and it took a rendering bug to establish it. The app used
/// to decide twice: once for the whole window, in cells, which is what tmux was
/// told — and again per pane, by measuring that pane's pixels, which is what the
/// emulator was actually sized to. The two disagreed, tmux painted only the grid
/// it believed in, and the cells outside it kept whatever a reflow had left there
/// — stray characters that survived every repaint, because every repaint agreed
/// with them, and cleared only when the pane re-attached under a new layout.
///
/// So these tests are about a relationship, not a number. Nothing here asserts
/// that a 1400×900 window is 174 columns; it asserts that measuring a pane's
/// rectangle is not a way to learn its grid, and that the emulator ends up
/// holding tmux's answer regardless of what its own pixels would have said.
@MainActor
struct PaneGridTests {
    /// The font the view will measure itself with, so the test and the view
    /// cannot disagree about what a cell is.
    private var cell: CGSize { TerminalMetrics.cell(Preferences.shared.terminalFont()) }

    private func rect(
        _ id: String, left: Int, top: Int, columns: Int, rows: Int
    ) -> PaneRect {
        PaneRect(
            id: id, short: id, title: nil, left: left, top: top, columns: columns, rows: rows,
            focused: false, zoomed: false)
    }

    /// An L: one pane down the left at full height, two stacked on the right.
    ///
    /// The shape that makes the disagreement systematic rather than a rounding
    /// accident. The window arithmetic charges every pane for `down` headers
    /// because that is the worst case; the tall pane on the left has exactly one
    /// header, so its rectangle carries the height of the ones it never spends —
    /// and measuring that rectangle hands it rows tmux never gave it.
    private func lShape(in window: PaneGrid) -> (tall: PaneRect, all: [PaneRect]) {
        let split = window.columns / 2
        let half = window.rows / 2
        let tall = rect("tall", left: 0, top: 0, columns: split, rows: window.rows)
        return (
            tall,
            [
                tall,
                rect("upper", left: split + 1, top: 0, columns: window.columns - split - 1, rows: half),
                rect(
                    "lower", left: split + 1, top: half + 1,
                    columns: window.columns - split - 1, rows: window.rows - half - 1),
            ]
        )
    }

    /// The claim the fix rests on: a pane's rectangle does not carry its grid.
    ///
    /// Scanned across a range of window sizes rather than asserted at one,
    /// because the point is not that some particular size is wrong — it is that
    /// you cannot pick a size and be safe. If this ever passes with no
    /// disagreement anywhere in the range, measuring would have become sound and
    /// this whole mechanism could go; that is worth being told about.
    @Test func measuringAPanesPixelsDoesNotRecoverTheGridTmuxSplit() {
        var disagreements = 0
        var compared = 0

        for width in stride(from: 900.0, through: 1800.0, by: 25.0) {
            for height in stride(from: 600.0, through: 1100.0, by: 25.0) {
                let size = CGSize(width: width, height: height)
                guard
                    let window = TileGeometry.viewport(
                        fitting: size, across: 2, down: 2, cell: cell)
                else { continue }
                let (tall, _) = lShape(in: window)

                let frame = TileGeometry.frame(of: tall, in: window, size: size)
                // What the terminal view inside the pane card is given: the
                // pane's rectangle, less the header above it.
                let surface = CGSize(
                    width: frame.width, height: frame.height - WorkspaceStyle.paneHeaderHeight)
                guard let measured = TileGeometry.fitting(surface, cell: cell) else { continue }

                compared += 1
                if measured != PaneGrid(columns: tall.columns, rows: tall.rows) {
                    disagreements += 1
                }
            }
        }

        #expect(compared > 0, "the scan produced no comparable sizes at all")
        #expect(
            disagreements > 0,
            """
            Measuring a pane's pixels agreed with tmux at every one of \(compared) \
            window sizes. Either the chrome arithmetic changed so that it now round-trips, \
            or this scan stopped exercising the L-shape. Check before deleting anything: \
            `TerminalRenderView.paneGrid` exists only because this disagrees.
            """)
    }

    /// And the fix: told a grid, the emulator holds that grid.
    ///
    /// Deliberately given a frame whose own measurement is something else, so a
    /// pass cannot come from the two happening to agree.
    @Test func aPaneToldItsGridHoldsItWhateverItsPixelsMeasure() {
        let size = CGSize(width: 1400, height: 900)
        guard let window = TileGeometry.viewport(fitting: size, across: 2, down: 2, cell: cell)
        else {
            Issue.record("no viewport fits 1400×900, which is a normal window")
            return
        }
        let (tall, _) = lShape(in: window)
        let frame = TileGeometry.frame(of: tall, in: window, size: size)

        let view = TerminalRenderView()
        view.frame = CGRect(
            x: 0, y: 0, width: frame.width,
            height: frame.height - WorkspaceStyle.paneHeaderHeight)
        view.layoutSubtreeIfNeeded()

        let tmux = PaneGrid(columns: tall.columns, rows: tall.rows)
        view.setPaneGrid(tmux)
        #expect(view.grid == tmux)

        // And a later layout pass does not quietly take it back. `layout()` runs
        // constantly — every divider drag, every window resize — and it is the
        // call that used to overwrite the grid with a measurement.
        view.layoutSubtreeIfNeeded()
        view.setFrameSize(CGSize(width: view.frame.width, height: view.frame.height + 40))
        view.layoutSubtreeIfNeeded()
        #expect(view.grid == tmux, "a layout pass resized the emulator behind tmux's back")
    }

    /// A narrow pane is held at its own width, not rounded up to a comfortable one.
    ///
    /// Five panes across a window with the sidebar open is about fourteen
    /// columns each, and the emulator used to clamp anything under twenty up to
    /// twenty. That put six columns in the grid that tmux does not believe it
    /// owns — the same defect as measuring the pixels, reached from the other
    /// end, and this test is here because the L-shape scan above cannot see it.
    @Test func aNarrowPaneIsHeldAtItsOwnWidthRatherThanARoundedUpOne() {
        let view = TerminalRenderView()
        view.frame = CGRect(x: 0, y: 0, width: 400, height: 300)
        view.layoutSubtreeIfNeeded()

        let narrow = PaneGrid(columns: 14, rows: 3)
        view.setPaneGrid(narrow)
        #expect(view.grid == narrow)
    }

    /// A click below the last row does not report a row the program does not have.
    ///
    /// The grid is tmux's and the view is the layout's, so the view is a little
    /// taller than the terminal in it. That band is inside the view and outside
    /// the screen, and a mouse-aware program asked about a row past its own last
    /// one has been handed a coordinate that cannot mean anything.
    @Test func aClickPastTheLastRowIsReportedAsTheLastRow() {
        let view = TerminalRenderView()
        view.frame = CGRect(x: 0, y: 0, width: 900, height: 600)
        view.layoutSubtreeIfNeeded()

        // Deliberately shorter and narrower than the frame measures, which is
        // the situation every tiled pane is in.
        let tmux = PaneGrid(columns: 40, rows: 10)
        view.setPaneGrid(tmux)

        let deep = NSEvent.mouseEvent(
            with: .leftMouseDown,
            location: view.convert(CGPoint(x: 880, y: 580), to: nil),
            modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            eventNumber: 0, clickCount: 1, pressure: 1)
        guard let deep else {
            Issue.record("could not synthesise a mouse event")
            return
        }

        let point = view.cellForTesting(deep)
        #expect(point.row <= tmux.rows - 1, "reported row \(point.row) of a \(tmux.rows)-row screen")
        #expect(
            point.column <= tmux.columns - 1,
            "reported column \(point.column) of a \(tmux.columns)-column screen")
    }

    /// The fallback still works, because something has to size the single
    /// terminal that has no layout behind it yet.
    @Test func aPaneWithNoGridToldToItSizesItselfFromItsPixels() {
        let view = TerminalRenderView()
        view.frame = CGRect(x: 0, y: 0, width: 800, height: 600)
        view.layoutSubtreeIfNeeded()

        let expected = TileGeometry.fitting(view.bounds.size, cell: cell)
        #expect(view.grid == expected)
    }
}
