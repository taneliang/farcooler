import AppKit
import SwiftUI

/// A draggable boundary between panes.
///
/// Derived from the pane rectangles rather than stored, because tmux does not
/// report dividers — it reports panes, and a divider is the gap one pane leaves
/// against its neighbour. Every pane whose right edge is not the window's right
/// edge has one to its right; likewise below. Which means a column of three panes
/// beside a tall one produces three separate handles along the same line, and that
/// is correct rather than a duplicate: `resize-pane` moves a border relative to a
/// PANE, so the handle beside the middle pane resizes the middle pane.
struct PaneDivider: Identifiable {
    let terminal: String
    let side: TileDirection
    /// The strip you can grab, in points.
    let rect: CGRect
    /// Cells this border would have to move for the two panes it separates to
    /// be the same size. Zero when they already are, which is also what makes
    /// a double-click on an even divider do nothing rather than jitter.
    ///
    /// The two panes it separates — not every pane in the window. A divider is
    /// a boundary between exactly two things, and that is the question a
    /// double-click on one asks. Evening out the whole window is a different
    /// request with its own binding; doing that from a divider would rearrange
    /// panes the pointer was nowhere near.
    let evenBy: Int

    var id: String { "\(terminal)-\(side.rawValue)" }

    var isVertical: Bool { side == .right }
}

extension PaneGroup {
    /// Where the dividers are, in a view of this size.
    ///
    /// Empty while zoomed: a zoomed pane covers the others, so the boundaries are
    /// still there in tmux but there is nothing on screen for them to separate,
    /// and a handle floating over a full-screen terminal would only be a way to
    /// resize something you cannot see.
    func dividers(in size: CGSize, thickness: CGFloat = 14) -> [PaneDivider] {
        guard zoomed == nil, columns > 0, rows > 0, panes.count > 1 else { return [] }

        let scaleX = size.width / CGFloat(columns)
        let scaleY = size.height / CGFloat(rows)
        var found: [PaneDivider] = []

        for pane in panes {
            let x = CGFloat(pane.left) * scaleX
            let y = CGFloat(pane.top) * scaleY
            let width = CGFloat(pane.columns) * scaleX
            let height = CGFloat(pane.rows) * scaleY

            // `< columns` rather than `!= columns`: a pane ending at the window's
            // edge has no neighbour to its right, and the one cell tmux leaves for
            // a divider means the last pane ends exactly at the edge.
            if pane.left + pane.columns < columns {
                found.append(
                    PaneDivider(
                        terminal: pane.id,
                        side: .right,
                        rect: CGRect(
                            x: x + width - thickness / 2,
                            y: y,
                            width: thickness,
                            height: height),
                        evenBy: evenBy(pane, .right)))
            }
            if pane.top + pane.rows < rows {
                found.append(
                    PaneDivider(
                        terminal: pane.id,
                        side: .bottom,
                        rect: CGRect(
                            x: x,
                            y: y + height - thickness / 2,
                            width: width,
                            height: thickness),
                        evenBy: evenBy(pane, .bottom)))
            }
        }
        return found
    }

    /// The neighbour across a pane's border, and how far to move it to halve
    /// the space they share.
    ///
    /// Zero when there is no neighbour to speak of, which keeps the caller from
    /// having to care: a border with nothing identifiable on the other side is
    /// one a double-click should leave alone.
    private func evenBy(_ pane: PaneRect, _ side: TileDirection) -> Int {
        guard let other = neighbour(of: pane, on: side) else { return 0 }
        let mine = side == .right ? pane.columns : pane.rows
        let theirs = side == .right ? other.columns : other.rows
        return (mine + theirs) / 2 - mine
    }

    /// The pane on the far side of a border.
    ///
    /// One cell further along, because that is the cell tmux leaves for the
    /// divider itself. Where several panes start there — a column of three
    /// against one tall pane — the one sharing the most edge with this pane
    /// wins, which is the one a pointer on this stretch of the border is
    /// actually between.
    private func neighbour(of pane: PaneRect, on side: TileDirection) -> PaneRect? {
        switch side {
        case .right:
            return panes
                .filter { $0.left == pane.left + pane.columns + 1 }
                .max { shared($0, pane, acrossRows: true) < shared($1, pane, acrossRows: true) }
        case .bottom:
            return panes
                .filter { $0.top == pane.top + pane.rows + 1 }
                .max { shared($0, pane, acrossRows: false) < shared($1, pane, acrossRows: false) }
        case .left, .top:
            // Never built: every divider is named for the pane whose right or
            // bottom edge it is, so these two cannot reach here.
            return nil
        }
    }

    /// How many cells of border two panes have in common.
    private func shared(_ a: PaneRect, _ b: PaneRect, acrossRows: Bool) -> Int {
        let (aStart, aSize) = acrossRows ? (a.top, a.rows) : (a.left, a.columns)
        let (bStart, bSize) = acrossRows ? (b.top, b.rows) : (b.left, b.columns)
        return max(0, min(aStart + aSize, bStart + bSize) - max(aStart, bStart))
    }
}

/// The handles, laid over the panes.
///
/// A separate overlay rather than a border on each card, because a divider belongs
/// to two panes and drawing it on one of them would make the hit area depend on
/// which pane happened to be on top.
struct PaneDividers: View {
    let group: PaneGroup
    let size: CGSize
    /// Terminal, which border, how many cells to move it. Signed.
    ///
    /// Returns whether the request was accepted. See `DividerView.sent`.
    let onResize: (String, TileDirection, Int) -> Bool

    var body: some View {
        ForEach(group.dividers(in: size)) { divider in
            DividerHandle(
                divider: divider,
                pointsPerCell: divider.isVertical
                    ? size.width / CGFloat(max(group.columns, 1))
                    : size.height / CGFloat(max(group.rows, 1)),
                onResize: onResize
            )
            .frame(width: divider.rect.width, height: divider.rect.height)
            .offset(x: divider.rect.minX, y: divider.rect.minY)
        }
    }
}

/// The handle itself, as an `NSView`.
///
/// AppKit rather than a SwiftUI `DragGesture`, and not by preference: the SwiftUI
/// version hit-tested correctly — it highlighted on hover and took the resize
/// cursor — and then never fired `onChanged` for an actual drag. A divider that
/// lights up when you reach it and does nothing when you pull it is worse than no
/// divider.
///
/// The terminal beneath is an `NSView` too, so this also puts the two on the same
/// footing instead of asking SwiftUI to arbitrate between a gesture recognizer and
/// a view that wants every event it can get.
struct DividerHandle: NSViewRepresentable {
    let divider: PaneDivider
    /// Points per cell on this divider's axis.
    ///
    /// From the layout's own grid rather than from the font: this view maps
    /// `columns` cells onto its width, so that ratio is what a cell is worth HERE.
    /// Measuring the font would be right only when the view happens to be an exact
    /// multiple of the cell size.
    let pointsPerCell: CGFloat
    let onResize: (String, TileDirection, Int) -> Bool

    func makeNSView(context: Context) -> DividerView {
        let view = DividerView()
        apply(to: view)
        return view
    }

    func updateNSView(_ view: DividerView, context: Context) {
        apply(to: view)
    }

    private func apply(to view: DividerView) {
        view.vertical = divider.isVertical
        view.pointsPerCell = pointsPerCell
        view.onResize = { cells in onResize(divider.terminal, divider.side, cells) }
        // The same resize the drag sends, with the arithmetic already done.
        // A double-click on a divider means "make these two the same" in every
        // app that has ever had one, and it is the gesture nobody has to be
        // told about.
        view.onEven = divider.evenBy == 0
            ? nil
            : { _ = onResize(divider.terminal, divider.side, divider.evenBy) }
    }
}

final class DividerView: NSView {
    var vertical = true
    var pointsPerCell: CGFloat = 8
    var onResize: ((Int) -> Bool)?
    /// Even out the two panes this divider separates. Nil when they already
    /// are, so a double-click on a centred divider is a no-op rather than a
    /// request the daemon has to refuse.
    var onEven: (() -> Void)?

    /// Cells the layout has actually been moved by in this drag.
    ///
    /// The gesture knows a total travel and `resize-pane` takes a relative amount,
    /// so what goes out is the difference. Crucially this only advances when the
    /// request was ACCEPTED: resizes are serialized, so one arriving while another
    /// is still in flight is dropped, and counting a dropped request as sent threw
    /// its cells away for good. Over a fast drag that lost most of them, and the
    /// divider trailed the pointer by a fraction that got worse the faster you
    /// moved — which is precisely what it looked like.
    ///
    /// Left where it is on a refusal, the next mouse event simply asks for
    /// everything still owed, so the divider catches up rather than falling behind.
    private var sent = 0
    private var origin = NSPoint.zero
    private var hovering = false { didSet { needsDisplay = true } }
    private var dragging = false { didSet { needsDisplay = true } }

    override var acceptsFirstResponder: Bool { false }
    // The pane underneath must not steal the click that starts a drag.
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: vertical ? .resizeLeftRight : .resizeUpDown)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(
            NSTrackingArea(
                rect: bounds,
                options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
                owner: self))
    }

    override func mouseEntered(with event: NSEvent) { hovering = true }
    override func mouseExited(with event: NSEvent) { hovering = false }

    override func mouseDown(with event: NSEvent) {
        // Handled on the second DOWN rather than on the second UP, which is
        // what makes it feel immediate — and no drag is started for it, so the
        // few points the hand moves between the two clicks cannot also nudge
        // the border it was about to centre.
        if event.clickCount == 2 {
            dragging = false
            onEven?()
            return
        }
        origin = event.locationInWindow
        sent = 0
        dragging = true
    }

    override func mouseDragged(with event: NSEvent) {
        guard dragging, pointsPerCell > 0 else { return }
        let now = event.locationInWindow
        // The window's y grows upward and a layout's rows grow downward, so a
        // downward drag is a negative dy here and has to be flipped to mean "move
        // the bottom border down".
        let travelled = vertical ? now.x - origin.x : origin.y - now.y
        let wanted = Int((travelled / pointsPerCell).rounded())
        guard wanted != sent else { return }
        if onResize?(wanted - sent) == true {
            sent = wanted
        }
    }

    override func mouseUp(with event: NSEvent) {
        dragging = false
        sent = 0
    }

    override func draw(_ dirtyRect: NSRect) {
        guard hovering || dragging else { return }
        NSColor.controlAccentColor.withAlphaComponent(dragging ? 0.55 : 0.28).setFill()
        // Drawn as a line down the middle rather than the full hit strip: the
        // grabbable area is deliberately wider than the divider looks, and
        // painting all of it would make the gap appear to jump when the pointer
        // arrives.
        let thickness: CGFloat = 3
        let line =
            vertical
            ? NSRect(x: bounds.midX - thickness / 2, y: 0, width: thickness, height: bounds.height)
            : NSRect(x: 0, y: bounds.midY - thickness / 2, width: bounds.width, height: thickness)
        line.fill()
    }
}
