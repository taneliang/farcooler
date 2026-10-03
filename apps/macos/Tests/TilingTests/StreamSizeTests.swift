import AppKit
import Testing

@testable import Far_Cooler

/// Who resizes the emulator once the stream says what size its pane is.
///
/// Growing a pane used to wrap every row of the program's repaint. tmux
/// resized the pane, the program repainted for the new size at once, and those
/// bytes reached this view on the stream before the layout reply that told it
/// the size — measured on a local runner, ~60ms before `layout viewport` even
/// returned. The repaint was drawn into the old, smaller grid, and growing the
/// grid when the reply landed reflowed the wreckage rather than undoing it.
///
/// A runner now puts the pane's size in the stream, in front of the repaint,
/// and the core resizes itself there (`farcooler_vt::size_marker`). These tests
/// are about the view's half: that it lets the stream do that, and that a
/// layout reply arriving afterwards cannot take the stream's size back.
@MainActor
struct StreamSizeTests {
    /// The bytes a runner's fanout writes when a pane becomes `columns`×`rows`.
    private func marker(_ columns: Int, _ rows: Int) -> [UInt8] {
        Array("\u{1b}P>farcooler-size;\(columns);\(rows)\u{1b}\\".utf8)
    }

    private func rows(of view: TerminalRenderView) -> [String] {
        view.core.withSnapshot { snapshot in
            (0..<snapshot.rows).map { row in
                String((0..<snapshot.columns).map { snapshot[row, $0].character ?? " " })
                    .replacingOccurrences(of: "\\s+$", with: "", options: .regularExpression)
            }
        } ?? []
    }

    /// The bug, as the view sees it: a pane grows, and its repaint arrives
    /// before any layout reply. It must land at the width it was written for.
    @Test func aRepaintForAGrownPaneLandsAtTheWidthItWasWrittenFor() {
        let view = TerminalRenderView()
        view.setPaneGrid(PaneGrid(columns: 20, rows: 4))

        let wide = String(repeating: "W", count: 40)
        view.feed(marker(40, 6) + Array("\u{1b}[H\u{1b}[2J\(wide)".utf8))

        #expect(view.grid == PaneGrid(columns: 40, rows: 6))
        let shown = rows(of: view)
        #expect(shown.first == wide, "the repaint wrapped: \(shown)")
        #expect(shown.dropFirst().first == "", "a full-width row spilled onto the next")
    }

    /// The layout reply lands after the stream has already resized the core.
    /// Usually it agrees and does nothing; when it is stale — a drag sending
    /// sizes faster than replies come back — it would size the grid away from
    /// the pane the program is drawing for, and the next repaint would wrap.
    @Test func aLayoutReplyCannotTakeTheStreamsSizeBack() {
        let view = TerminalRenderView()
        view.setPaneGrid(PaneGrid(columns: 20, rows: 4))
        view.feed(marker(40, 6) + Array("x".utf8))

        view.setPaneGrid(PaneGrid(columns: 30, rows: 5))
        #expect(view.grid == PaneGrid(columns: 40, rows: 6), "a late layout reply resized the grid")
    }

    /// A runner too old to send sizes never says, and then the layout is all
    /// there is: the view must keep resizing on its word.
    @Test func withoutSizesInTheStreamTheLayoutStillSizesTheGrid() {
        let view = TerminalRenderView()
        view.setPaneGrid(PaneGrid(columns: 20, rows: 4))
        view.feed(Array("no marker here".utf8))

        view.setPaneGrid(PaneGrid(columns: 30, rows: 5))
        #expect(view.grid == PaneGrid(columns: 30, rows: 5))
    }

    /// A kept frame was sized by a stream that has gone. Shown again in a pane
    /// of another size, it follows that pane until its new stream speaks.
    @Test func aKeptFrameFollowsTheLayoutUntilItsNewStreamSpeaks() {
        let leaving = TerminalRenderView()
        leaving.setPaneGrid(PaneGrid(columns: 20, rows: 4))
        leaving.feed(marker(40, 6) + Array("kept".utf8))

        let arriving = TerminalRenderView()
        arriving.setPaneGrid(PaneGrid(columns: 40, rows: 6))
        arriving.showRetained(leaving.core)
        arriving.setPaneGrid(PaneGrid(columns: 50, rows: 8))
        #expect(arriving.grid == PaneGrid(columns: 50, rows: 8))
    }
}
