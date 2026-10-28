import AppKit
import Testing

@testable import Far_Cooler

/// Coming back to the app passes the terminal area through a sliver while the
/// window and sidebar settle. A sliver that was raised to a floor and sent
/// told tmux the window was 20 columns, which it split between the panes: the
/// claude TUI reflowed to a few characters a line until the real size followed
/// (ov-853).
@MainActor
struct ActivationSizeTests {
    private var cell: CGSize { TerminalMetrics.cell(Preferences.shared.terminalFont()) }

    @Test func noTinySizeIsSentWhileTheLayoutSettles() {
        let settled = CGSize(width: 1400, height: 900)
        // Activation: the settled frame, a sliver, a narrow strip, the settled frame.
        let frames = [settled, CGSize(width: 40, height: 30), CGSize(width: 120, height: 900), settled]
        let sent = frames.compactMap {
            TileGeometry.viewport(fitting: $0, across: 2, down: 1, cell: cell)
        }
        #expect(sent.count == 2, "only the settled frames are sent")
        for grid in sent {
            #expect(grid.columns >= 2 * TileGeometry.minimumColumns)
            #expect(grid.rows >= TileGeometry.minimumRows)
        }
    }

    @Test func aLoneTerminalDoesNotReportASliver() {
        #expect(TileGeometry.fitting(CGSize(width: 60, height: 40), cell: cell) == nil)
        #expect(TileGeometry.fitting(CGSize(width: 900, height: 600), cell: cell) != nil)
    }
}
