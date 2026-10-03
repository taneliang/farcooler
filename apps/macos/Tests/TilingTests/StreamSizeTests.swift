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
/// and the core resizes itself there (`farcooler_vt::size_marker`) — when the
/// view trusts the runner (`StreamSizes`). These tests are about the view's
/// half: that it lets a trusted stream do that, that a layout reply a marker
/// overtook cannot take the size back, and that a reply no marker follows is
/// still applied.
@MainActor
struct StreamSizeTests {
    /// The bytes a runner's fanout writes when a pane becomes `columns`×`rows`.
    private func marker(_ columns: Int, _ rows: Int) -> [UInt8] {
        Array("\u{1b}P>farcooler-size;\(columns);\(rows)\u{1b}\\".utf8)
    }

    private func trusting(_ grid: PaneGrid) -> TerminalRenderView {
        let view = TerminalRenderView()
        view.trustStreamSizes(true)
        view.setPaneGrid(grid)
        return view
    }

    /// Long enough for any fallback to have fired.
    private func pastTheFallback() async {
        try? await Task.sleep(for: TerminalRenderView.sizeFallbackDelay * 4)
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
        let view = trusting(PaneGrid(columns: 20, rows: 4))

        let wide = String(repeating: "W", count: 40)
        view.feed(marker(40, 6) + Array("\u{1b}[H\u{1b}[2J\(wide)".utf8))

        #expect(view.grid == PaneGrid(columns: 40, rows: 6))
        let shown = rows(of: view)
        #expect(shown.first == wide, "the repaint wrapped: \(shown)")
        #expect(shown.dropFirst().first == "", "a full-width row spilled onto the next")
    }

    /// A reply that lands just after a marker may be stale — a drag sending
    /// sizes faster than replies come back — and would size the grid away from
    /// the pane the program is drawing for. The marker wins, even after the
    /// fallback's wait.
    @Test func aLayoutReplyAMarkerOvertookCannotTakeTheSizeBack() async {
        let view = trusting(PaneGrid(columns: 20, rows: 4))
        view.feed(marker(40, 6) + Array("x".utf8))

        view.setPaneGrid(PaneGrid(columns: 30, rows: 5))
        #expect(view.grid == PaneGrid(columns: 40, rows: 6), "a late layout reply resized the grid")
        await pastTheFallback()
        #expect(view.grid == PaneGrid(columns: 40, rows: 6), "the fallback overrode the stream")
    }

    /// No marker comes for a resize nobody writes after, or from a runner whose
    /// fanout lost its pane's tty. The layout's size lands anyway, a moment late.
    @Test func aLayoutReplyNoMarkerFollowsIsStillApplied() async {
        let view = trusting(PaneGrid(columns: 20, rows: 4))
        view.feed(marker(40, 6) + Array("x".utf8))
        try? await Task.sleep(for: TerminalRenderView.sizeFallbackDelay * 2)

        view.setPaneGrid(PaneGrid(columns: 60, rows: 10))
        await pastTheFallback()
        #expect(
            view.grid == PaneGrid(columns: 60, rows: 10), "a resize with no marker never landed")
    }

    /// A stream from a runner that made no promise is just bytes: a marker in
    /// it — `farcooler terminal stream` run inside the pane prints them — must
    /// not size the grid, and the layout keeps doing it.
    @Test func anUntrustedStreamsMarkersAreIgnored() {
        let view = TerminalRenderView()
        view.setPaneGrid(PaneGrid(columns: 20, rows: 4))
        view.feed(marker(40, 6) + Array("x".utf8))
        #expect(view.grid == PaneGrid(columns: 20, rows: 4))

        view.setPaneGrid(PaneGrid(columns: 30, rows: 5))
        #expect(view.grid == PaneGrid(columns: 30, rows: 5))
    }

    /// A runner too old to send sizes never says, and then the layout is all
    /// there is: the view must keep resizing on its word, at once.
    @Test func withoutSizesInTheStreamTheLayoutStillSizesTheGrid() {
        let view = trusting(PaneGrid(columns: 20, rows: 4))
        view.feed(Array("no marker here".utf8))

        view.setPaneGrid(PaneGrid(columns: 30, rows: 5))
        #expect(view.grid == PaneGrid(columns: 30, rows: 5))
    }

    /// A kept frame was sized by a stream that has gone. Shown again in a pane
    /// of another size, it follows that pane until its new stream speaks.
    @Test func aKeptFrameFollowsTheLayoutUntilItsNewStreamSpeaks() {
        let leaving = trusting(PaneGrid(columns: 20, rows: 4))
        leaving.feed(marker(40, 6) + Array("kept".utf8))

        let arriving = trusting(PaneGrid(columns: 40, rows: 6))
        arriving.showRetained(leaving.core)
        arriving.setPaneGrid(PaneGrid(columns: 50, rows: 8))
        #expect(arriving.grid == PaneGrid(columns: 50, rows: 8))
    }
}
