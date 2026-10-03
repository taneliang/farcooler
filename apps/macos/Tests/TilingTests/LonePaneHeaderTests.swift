import AppKit
import Testing

@testable import Far_Cooler

/// A lone pane draws no header, and tmux is told about the rows that frees
/// (ov-214).
///
/// The header is subtracted from the window's grid once per pane down a
/// column (`TileGeometry.viewport`). Charged for a header it no longer
/// draws, a lone pane would be given about two rows fewer than it shows:
/// empty rows at its foot that nothing paints.
@MainActor
struct LonePaneHeaderTests {
    private var cell: CGSize { TerminalMetrics.cell(Preferences.shared.terminalFont()) }

    @Test("Only a pane beside others, or a Changes pane, draws a header")
    func whoDrawsAHeader() {
        #expect(!TileGeometry.showsHeader(panes: 1, isChanges: false))
        #expect(TileGeometry.showsHeader(panes: 1, isChanges: true))
        #expect(TileGeometry.showsHeader(panes: 2, isChanges: false))
        #expect(TileGeometry.showsHeader(panes: 4, isChanges: false))
    }

    @Test("A lone pane without a header is asked for the rows the header took")
    func aLonePaneGetsTheRows() throws {
        let size = CGSize(width: 1400, height: 900)
        let with = try #require(TileGeometry.viewport(fitting: size, across: 1, down: 1, cell: cell))
        let without = try #require(TileGeometry.viewport(fitting: size, across: 1, down: 1, cell: cell, header: 0))
        #expect(without.columns == with.columns)
        #expect(without.rows > with.rows)
        // Exactly what the renderer fits in the pane with its insets alone.
        #expect(without == TileGeometry.fitting(size, cell: cell))
    }

    @Test("Two panes stacked still pay for two headers")
    func twoStackedPayTwice() throws {
        let size = CGSize(width: 1400, height: 900)
        let two = try #require(TileGeometry.viewport(fitting: size, across: 1, down: 2, cell: cell))
        let usable = size.height - 2 * (TerminalMetrics.padding.top + TerminalMetrics.padding.bottom)
            - 2 * WorkspaceStyle.paneHeaderHeight
        #expect(two.rows == max(5, Int(usable / cell.height)))
    }

    private func group(_ ids: [String]) -> PaneGroup {
        let panes = ids.enumerated().map { index, id in
            PaneRect(
                id: id, short: id, title: nil, left: 0, top: index * 25, columns: 80, rows: 24,
                focused: false, zoomed: false)
        }
        return PaneGroup(id: "@1", name: "", active: true, columns: 80, rows: 49, layout: "a", panes: panes)
    }

    private func terminal(_ id: String, changes: Bool = false) -> Terminal {
        var terminal = Terminal(id: id, short: id, title: id, preset: "shell", state: "running", epoch: 0)
        if changes { terminal.paneMode = "changes" }
        return terminal
    }

    @Test("The tile view charges each layout for the header its panes draw")
    func theTileViewChargesWhatItDraws() {
        #expect(TileView.headerHeight(group(["a"]), terminals: [terminal("a")]) == 0)
        #expect(
            TileView.headerHeight(group(["a"]), terminals: [terminal("a", changes: true)])
                == WorkspaceStyle.paneHeaderHeight)
        #expect(
            TileView.headerHeight(group(["a", "b"]), terminals: [terminal("a"), terminal("b")])
                == WorkspaceStyle.paneHeaderHeight)
    }
}
