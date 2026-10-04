import AppKit
import CFarCoolerVT

/// What a terminal stops doing when nobody can see it, and how little it
/// redraws when somebody can (ov-229).
///
/// Every pane ran a display link at the display's refresh rate for as long as
/// it was in a window, and redrew its whole grid on every new byte: a full
/// snapshot, a background pass and a glyph pass over every cell. An agent's
/// spinner is about eight new frames a second, so each working pane redrew
/// its whole screen eight times a second. That happened in a window behind
/// others, with the app hidden, and for panes zoomed out of sight at opacity
/// zero.
///
/// So the link pauses while the pane can't be seen. Bytes still reach the
/// emulator, and its replies still go back, but nothing draws. When the pane
/// is shown again, the first tick finds the revision moved and draws once.
/// When it is drawn, only the rows that changed are redrawn.
extension TerminalRenderView {
    /// Whether this pane should draw: shown by its layout, in a window
    /// somebody can see.
    var drawsNow: Bool { isShown && windowWatch.isVisible }

    /// Pause or resume the display link to match `drawsNow`.
    func updateDrawing() {
        restingTicks = 0
        displayLink?.isPaused = !drawsNow
    }

    /// How many empty ticks the link runs before it rests: half a second at
    /// 120 Hz. Longer than the 150 ms a synchronized update may stay open, so
    /// a program that opened one and died is still flushed by a tick
    /// (`VTCore.flushExpiredSync`): its bytes woke the link, and the link
    /// outlives the deadline.
    static let ticksBeforeResting = 60

    /// A tick found nothing new. After enough of them the link rests until
    /// the screen moves again (`wake`). A visible pane with a quiet program
    /// was otherwise woken 120 times a second to compare one integer, which
    /// was most of what a visible idle window still cost.
    func rest() {
        restingTicks += 1
        if restingTicks >= Self.ticksBeforeResting { displayLink?.isPaused = true }
    }

    /// The screen may have moved: tick again, if anybody can see it.
    func wake() {
        guard drawsNow else { return }
        restingTicks = 0
        displayLink?.isPaused = false
    }

    /// Mark for redrawing only the rows that differ from the last frame.
    ///
    /// Falls back to the whole view whenever the comparison can't say: the
    /// first frame, a new size, a scroll into history.
    func invalidateChangedRows() {
        let changed = core.withSnapshot { damage.rows(changedIn: $0) } ?? nil
        guard let changed else {
            needsDisplay = true
            return
        }
        let rows = core.withSnapshot { $0.rows } ?? 0
        for row in TerminalDamage.withNeighbors(changed, rows: rows) { setNeedsDisplay(rowRect(row)) }
    }
}

/// Which rows of a terminal changed between two frames.
///
/// A copy of the last frame's cells, compared row by row: a `memcmp` of a few
/// kilobytes per row. That costs far less than drawing the rows that didn't
/// change. The cursor counts as part of the rows it leaves and enters.
struct TerminalDamage {
    private var cells: [FarCoolerVtCell] = []
    private var columns = 0
    private var rows = 0
    private var displayOffset = -1
    private var cursor: (row: Int, column: Int, visible: Bool) = (-1, -1, false)

    /// The changed rows and the one above and below each, within `0..<rows`.
    ///
    /// A glyph can be taller than its row (an emoji drawn from a fallback
    /// font), so a redraw clipped to the exact strip could cut its tail off,
    /// or leave a fragment of the one it replaced in the next row.
    static func withNeighbors(_ changed: IndexSet, rows: Int) -> IndexSet {
        var padded = IndexSet()
        for row in changed {
            for neighbor in (row - 1)...(row + 1) where (0..<rows).contains(neighbor) { padded.insert(neighbor) }
        }
        return padded
    }

    /// The rows that changed since the last call, or nil for "all of them".
    mutating func rows(changedIn snapshot: VTSnapshot) -> IndexSet? {
        defer { remember(snapshot) }
        guard snapshot.columns == columns, snapshot.rows == rows, snapshot.displayOffset == displayOffset,
            cells.count == snapshot.cells.count, columns > 0
        else { return nil }

        var changed = IndexSet()
        let rowBytes = columns * MemoryLayout<FarCoolerVtCell>.stride
        cells.withUnsafeBytes { old in
            let new = UnsafeRawBufferPointer(snapshot.cells)
            guard let oldBase = old.baseAddress, let newBase = new.baseAddress else { return }
            for row in 0..<rows where memcmp(oldBase + row * rowBytes, newBase + row * rowBytes, rowBytes) != 0 {
                changed.insert(row)
            }
        }
        let now = (row: snapshot.cursorRow, column: snapshot.cursorColumn, visible: snapshot.cursorVisible)
        if now != cursor {
            for row in [cursor.row, now.row] where (0..<rows).contains(row) { changed.insert(row) }
        }
        return changed
    }

    private mutating func remember(_ snapshot: VTSnapshot) {
        columns = snapshot.columns
        rows = snapshot.rows
        displayOffset = snapshot.displayOffset
        cursor = (snapshot.cursorRow, snapshot.cursorColumn, snapshot.cursorVisible)
        cells.removeAll(keepingCapacity: true)
        cells.append(contentsOf: snapshot.cells)
    }
}
