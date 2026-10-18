import UIKit

// Select and Copy on a terminal pane (ov-609), as iOS terminal apps do it
// (Blink, Termius): a long press on text selects the word under it, the drag
// that follows or either handle moves an end, a handle held at the top or
// bottom edge scrolls this device's view into the scrollback (never the
// program's: on the alternate screen it doesn't scroll), and the edit menu
// offers Copy.
//
// Paste isn't beside Copy: a menu action of the app's own that reads the
// pasteboard makes the system ask "Allow Paste?" each time, where ov-604's
// Paste goes through the responder's `paste(_:)` and never asks (review 1).
// A press on empty terminal is still where Paste is.
//
// The gesture map on the pane, one press each:
//   a link             its actions (ov-215), unchanged
//   text               the word selected, handles, Copy
//   empty terminal     Paste (ov-604), unchanged
//   a tap              clears a selection; otherwise the keyboard, unchanged
//
// The selection is cells (`TerminalSelection`), counted from the oldest line
// of scrollback, so it keeps its place while the pane scrolls. This class
// owns it, draws it over the grid, and moves its ends; what it doesn't know
// (which cell a point is, where a cell is drawn, the text of a range) it asks
// the terminal view through `TerminalSelectionHooks`.

/// What a selection asks of the terminal view it sits on.
struct TerminalSelectionHooks {
    /// The cell under a point in the view, clamped onto the grid.
    var point: (CGPoint) -> TerminalSelection.Point = { _ in TerminalSelection.Point(line: 0, column: 0) }
    /// The word under a point, nil on a blank.
    var word: (CGPoint) -> TerminalSelection? = { _ in nil }
    /// Where a cell is drawn in the view, nil while it's scrolled off.
    var rect: (TerminalSelection.Point) -> CGRect? = { _ in nil }
    /// The grid's width in columns.
    var columns: () -> Int = { 0 }
    /// How many lines of scrollback there are now.
    var history: () -> Int = { 0 }
    /// The text of a range, read from the emulator, scrollback included.
    var text: (TerminalSelection) -> String = { _ in "" }
    /// Scroll by lines, positive into the scrollback.
    var scroll: (Int) -> Void = { _ in }
}

@MainActor
final class TerminalSelectionController: NSObject, UIEditMenuInteractionDelegate, UIGestureRecognizerDelegate {
    var hooks = TerminalSelectionHooks()
    private(set) var selection: TerminalSelection?
    private weak var host: UIView?
    private let drawn = TerminalSelectionView()
    private lazy var menu = UIEditMenuInteraction(delegate: self)
    private let drag = UIPanGestureRecognizer()
    /// The end a handle drag holds still.
    private var fixed: TerminalSelection.Point?
    private var lastDrag = CGPoint.zero
    private var edge: Timer?
    /// Whether the press under way is the one that made the selection: only
    /// that press's drag moves its end, never a press that opened a link or
    /// Paste with an old selection still up.
    private var pressing = false
    /// The scrollback's length when the selection was made. Fewer lines now
    /// means the emulator was replaced or reset (a recapture), and the lines
    /// the selection counts are other lines: it's put away.
    private var history = 0

    /// The distance from a handle a touch may land and still take it.
    private static let reach: CGFloat = 24

    var isActive: Bool { selection != nil }

    func install(on view: UIView, scrollPan: UIPanGestureRecognizer) {
        host = view
        drawn.frame = view.bounds
        drawn.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        drawn.isUserInteractionEnabled = false
        view.addSubview(drawn)
        view.addInteraction(menu)
        drag.addTarget(self, action: #selector(handleDrag(_:)))
        drag.delegate = self
        view.addGestureRecognizer(drag)
        // A drag that starts on a handle moves the handle, never the pane.
        scrollPan.require(toFail: drag)
    }

    /// A long press away from a link: true when it landed on a word, which
    /// is now selected.
    func begin(at point: CGPoint) -> Bool {
        guard let word = hooks.word(point) else { return false }
        menu.dismissMenu()
        selection = word
        fixed = word.start
        history = hooks.history()
        pressing = true
        redraw()
        return true
    }

    /// The press that made the selection, still down and moving.
    func pressMoved(to point: CGPoint) {
        guard pressing, let host else { return }
        lastDrag = point
        extend(to: point)
        followEdge(point, in: host)
    }

    /// The press lifted: its menu, if it made a selection.
    func pressEnded() {
        edge?.invalidate()
        edge = nil
        if pressing, selection != nil { showMenu() }
        pressing = false
    }

    /// The finger still down after the press, moving: the far end follows.
    func extend(to point: CGPoint) {
        guard let fixed, selection != nil else { return }
        selection = TerminalSelection(anchor: fixed, head: hooks.point(point))
        redraw()
    }

    func showMenu() {
        guard let selection, let host else { return }
        let box = bounds(of: selection) ?? CGRect(origin: lastDrag, size: .zero)
        let at = CGPoint(x: min(max(box.midX, 0), host.bounds.width), y: max(box.minY, 0))
        menu.presentEditMenu(with: UIEditMenuConfiguration(identifier: nil, sourcePoint: at))
    }

    func clear() {
        guard selection != nil else { return }
        selection = nil
        fixed = nil
        pressing = false
        menu.dismissMenu()
        redraw()
    }

    func copy() {
        guard let selection else { return }
        UIPasteboard.general.string = hooks.text(selection)
        clear()
    }

    /// Drawn again where the pane now shows it: after a scroll, a resize, a
    /// new screen.
    func redraw() {
        if selection != nil, hooks.history() < history { return clear() }
        guard let selection else {
            drawn.update(rects: [], start: nil, end: nil)
            return
        }
        let columns = hooks.columns()
        var rects: [CGRect] = []
        for line in selection.start.line...selection.end.line {
            guard let range = selection.columns(on: line, width: columns),
                let first = hooks.rect(TerminalSelection.Point(line: line, column: range.lowerBound)),
                let last = hooks.rect(TerminalSelection.Point(line: line, column: range.upperBound))
            else { continue }
            rects.append(first.union(last))
        }
        drawn.update(rects: rects, start: hooks.rect(selection.start), end: hooks.rect(selection.end))
    }

    // MARK: - The menu

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction, menuFor configuration: UIEditMenuConfiguration,
        suggestedActions: [UIMenuElement]
    ) -> UIMenu? {
        guard selection != nil else { return nil }
        return UIMenu(children: [
            UIAction(title: "Copy", image: UIImage(systemName: "doc.on.doc")) { [weak self] _ in self?.copy() }
        ])
    }

    func editMenuInteraction(_ interaction: UIEditMenuInteraction, targetRectFor configuration: UIEditMenuConfiguration) -> CGRect {
        selection.flatMap(bounds(of:)) ?? .null
    }

    // MARK: - The handles

    func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
        guard recognizer === drag, let selection, let host else { return false }
        return handle(near: recognizer.location(in: host), of: selection) != nil
    }

    func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        false
    }

    @objc private func handleDrag(_ recognizer: UIPanGestureRecognizer) {
        guard let host else { return }
        let point = recognizer.location(in: host)
        lastDrag = point
        switch recognizer.state {
        case .began:
            guard let selection, let end = handle(near: point, of: selection) else { return }
            fixed = end == selection.start ? selection.end : selection.start
            menu.dismissMenu()
        case .changed:
            extend(to: point)
            followEdge(point, in: host)
        case .ended, .cancelled, .failed:
            edge?.invalidate()
            edge = nil
            showMenu()
        default:
            break
        }
    }

    /// Held at the top or bottom edge, the pane scrolls a line every tenth
    /// of a second and the handle's end goes with it, into the scrollback
    /// or back out.
    private func followEdge(_ point: CGPoint, in host: UIView) {
        let band: CGFloat = 28
        let lines = point.y < band ? 1 : (point.y > host.bounds.height - band ? -1 : 0)
        guard lines != 0 else {
            edge?.invalidate()
            edge = nil
            return
        }
        guard edge == nil else { return }
        edge = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let lines = self.lastDrag.y < band ? 1 : (self.lastDrag.y > (self.host?.bounds.height ?? 0) - band ? -1 : 0)
                guard lines != 0 else { return }
                self.hooks.scroll(lines)
                self.extend(to: self.lastDrag)
            }
        }
    }

    /// The end whose handle is within reach of `point`.
    private func handle(near point: CGPoint, of selection: TerminalSelection) -> TerminalSelection.Point? {
        func distance(_ rect: CGRect?, top: Bool) -> CGFloat {
            guard let rect else { return .infinity }
            let anchor = CGPoint(x: top ? rect.minX : rect.maxX, y: rect.midY)
            return hypot(point.x - anchor.x, point.y - anchor.y)
        }
        let toStart = distance(hooks.rect(selection.start), top: true)
        let toEnd = distance(hooks.rect(selection.end), top: false)
        guard min(toStart, toEnd) <= Self.reach else { return nil }
        return toStart <= toEnd ? selection.start : selection.end
    }

    private func bounds(of selection: TerminalSelection) -> CGRect? {
        let rects = drawn.rects
        guard var box = rects.first else { return nil }
        for rect in rects.dropFirst() { box = box.union(rect) }
        return box
    }
}

/// The selection drawn over the grid: a wash over each selected run of
/// cells, and a handle at each end, a bar the cell's height with a knob,
/// above the start and below the end, as a text view draws them.
final class TerminalSelectionView: UIView {
    private(set) var rects: [CGRect] = []
    private var start: CGRect?
    private var end: CGRect?

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        backgroundColor = .clear
        contentMode = .redraw
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    func update(rects: [CGRect], start: CGRect?, end: CGRect?) {
        self.rects = rects
        self.start = start
        self.end = end
        setNeedsDisplay()
    }

    override func draw(_ rect: CGRect) {
        guard !rects.isEmpty, let context = UIGraphicsGetCurrentContext() else { return }
        let tint = tintColor ?? .systemBlue
        context.setFillColor(tint.withAlphaComponent(0.3).cgColor)
        for run in rects { context.fill(run) }
        context.setFillColor(tint.cgColor)
        let knob: CGFloat = 10
        if let start {
            context.fill(CGRect(x: start.minX - 1, y: start.minY, width: 2, height: start.height))
            context.fillEllipse(in: CGRect(x: start.minX - knob / 2, y: start.minY - knob, width: knob, height: knob))
        }
        if let end {
            context.fill(CGRect(x: end.maxX - 1, y: end.minY, width: 2, height: end.height))
            context.fillEllipse(in: CGRect(x: end.maxX - knob / 2, y: end.maxY, width: knob, height: knob))
        }
    }
}

extension TerminalSelectionHooks {
    /// The hooks for a pane laid out as `layout` over `session`'s emulator.
    /// Each reads the session's grid and scroll position when it's called,
    /// not when the view last drew: an edge scroll moves the view and then
    /// extends the selection on the same turn, before SwiftUI draws again.
    @MainActor
    init(fallback grid: TerminalGrid, layout: TerminalGridLayout, session: TerminalSession) {
        func screen() -> (grid: TerminalGrid, offset: Int, history: Int) {
            let position = session.scrollPosition
            return (session.grid ?? grid, position.offset, position.history)
        }
        func cell(_ point: CGPoint, in grid: TerminalGrid) -> (row: Int, column: Int) {
            guard layout.cell.width > 0, layout.cell.height > 0 else { return (0, 0) }
            let column = Int(((point.x - layout.origin.x) / layout.cell.width).rounded(.down))
            let row = Int(((point.y - layout.origin.y) / layout.cell.height).rounded(.down))
            return (min(max(row, 0), max(grid.rows - 1, 0)), min(max(column, 0), max(grid.columns - 1, 0)))
        }
        self.init(
            point: { point in
                let now = screen()
                let at = cell(point, in: now.grid)
                return TerminalSelection.Point(
                    line: TerminalSelection.line(row: at.row, offset: now.offset, history: now.history), column: at.column)
            },
            word: { point in
                let now = screen()
                let grid = now.grid
                guard grid.rows > 0, grid.columns > 0 else { return nil }
                let at = cell(point, in: grid)
                let cells = (0..<grid.columns).map { TerminalSelection.Cell(grid[at.row, $0].character, wide: grid[at.row, $0].wide) }
                return TerminalSelection.word(
                    at: at.column, in: cells, line: TerminalSelection.line(row: at.row, offset: now.offset, history: now.history))
            },
            rect: { point in
                let now = screen()
                let row = TerminalSelection.row(line: point.line, offset: now.offset, history: now.history)
                guard row >= 0, row < now.grid.rows else { return nil }
                return CGRect(
                    x: layout.origin.x + CGFloat(point.column) * layout.cell.width,
                    y: layout.origin.y + CGFloat(row) * layout.cell.height,
                    width: layout.cell.width, height: layout.cell.height)
            },
            columns: { screen().grid.columns },
            history: { session.scrollPosition.history },
            text: { session.text(of: $0, columns: screen().grid.columns) },
            // This device's own view of the scrollback, and nothing else
            // (review 1): `TerminalSession.scroll` sends a wheel to a program
            // on the alternate screen, and a copy must never type into one.
            // The alternate screen has no scrollback, so there it does nothing.
            scroll: { lines in
                guard let vt = session.vt, !vt.isAlternateScreen else { return }
                session.scroll(lines: lines, column: 0, row: 0)
            })
    }
}

extension TerminalSession {
    /// The text of a selected range, read from the emulator rather than the
    /// grid on screen, so a range that runs into the scrollback copies whole.
    /// The core hands out the screen at its scroll position, so this moves
    /// the view to each screenful the range covers and back, on this one turn
    /// of the main actor: nothing is drawn in between.
    func text(of selection: TerminalSelection, columns: Int) -> String {
        guard let vt else { return "" }
        let (offset, history) = vt.scrollPosition
        var rows: [Int: [TerminalSelection.Cell]] = [:]
        func read() {
            _ = vt.withSnapshot { snapshot in
                for row in 0..<snapshot.rows {
                    let line = TerminalSelection.line(row: row, offset: snapshot.displayOffset, history: snapshot.historySize)
                    guard line >= selection.start.line, line <= selection.end.line, rows[line] == nil else { continue }
                    rows[line] = (0..<snapshot.columns).map { column in
                        let cell = snapshot[row, column]
                        return TerminalSelection.Cell(cell.character, wide: cell.isWide)
                    }
                }
            }
        }
        read()
        // Lines the screen doesn't hold: the view moved to put each at its
        // top (or as near as the scrollback allows), read, and put back.
        var at = offset
        for wanted in selection.start.line...selection.end.line where rows[wanted] == nil {
            let target = min(max(history - wanted, 0), history)
            guard target != at else { continue }
            vt.scroll(lines: Int32(target - at))
            at = target
            read()
        }
        if at != offset { vt.scroll(lines: Int32(offset - at)) }
        return selection.text(width: columns) { rows[$0] }
    }
}
