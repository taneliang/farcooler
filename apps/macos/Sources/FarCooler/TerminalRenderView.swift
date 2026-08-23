import AppKit
import CFarCoolerVT
import Combine
import CoreText

/// The cell grid a font produces, and the insets a pane draws inside.
///
/// Lifted out of the render view because two places now need it and they must
/// not disagree: the tiled view sizes the whole VIEWPORT from it, in cells, and
/// hands that to tmux, while the renderer positions every glyph with it — so if
/// the two measured cells differently, tmux would lay out for a grid the
/// renderer cannot draw and every pane would be a column or two out.
///
/// Agreeing about the CELL is necessary and was never sufficient. A pane's grid
/// is not derived from cells here at all any more; it is whatever tmux says it
/// is. See `TileGeometry` for why measuring it a second time was wrong.
enum TerminalMetrics {
    /// Insets so glyphs do not touch the pane edges.
    static let padding = NSEdgeInsets(top: 8, left: 10, bottom: 8, right: 10)

    /// One cell, measured rather than assumed: the user's font and size change it.
    static func cell(_ font: NSFont) -> CGSize {
        // Every cell is the same box, so one advance defines the grid.
        var glyph = CGGlyph()
        var ch: UniChar = 0x4D  // "M"
        CTFontGetGlyphsForCharacters(font, &ch, &glyph, 1)
        var advance = CGSize.zero
        CTFontGetAdvancesForGlyphs(font, .horizontal, &glyph, &advance, 1)
        return CGSize(
            width: advance.width,
            height: (font.ascender - font.descender + font.leading).rounded(.up))
    }
}

/// Draws the terminal and routes input to it.
///
/// The whole of this view's job is pixels and events. It does not parse an
/// escape sequence, does not know what an arrow key means, and does not decide
/// what color `\e[31m` is — the Rust core answers all of that, which is what
/// lets the same answers serve iOS and Android. What remains here is genuinely
/// platform work: fonts, glyph rasterisation, NSEvent, the pasteboard.
@MainActor
final class TerminalRenderView: NSView, NSUserInterfaceValidations {
    /// Encoded bytes leaving the terminal, ready for the pane.
    var onInput: (([UInt8]) -> Void)?
    /// An image was pasted or dropped. Handled above this view, because getting
    /// it onto the runner the pane is on is a daemon conversation, not drawing.
    var onPasteImage: ((PastedImage) -> Void)?
    /// The grid this view can show, which is what the pane must be resized to.
    var onGeometry: ((Int, Int) -> Void)?

    private(set) var core: VTCore
    private var displayLink: CADisplayLink?
    /// Watches the theme, so a pick in Settings reaches a live terminal.
    ///
    /// Subscribed here rather than threaded down as a `themeRevision` property
    /// beside `fontRevision`: that would mean a new argument on four view
    /// layers to deliver one integer to the one object that acts on it, and
    /// every one of those layers would be passing it through untouched.
    private var themeObserver: AnyCancellable?
    private var lastDrawnRevision: UInt64 = .max
    private var lastReportedGeometry = PaneGrid(columns: 0, rows: 0)
    /// The grid tmux says this pane has, once somebody knows it.
    ///
    /// A pane's size is tmux's answer, not ours. This view can measure how many
    /// cells fit in its own pixels, and in a tiled layout that measurement is
    /// NOT the pane: `TileView` tells tmux one grid for the whole window, tmux
    /// divides it, and each pane's share comes back through `list-panes`. The
    /// two round trips do not agree — a pane spanning two rows of the layout
    /// spends one header's worth of chrome while the window arithmetic charged
    /// it two, so measuring gave it a few rows more than it has.
    ///
    /// Those extra rows and columns are the bug this exists to close. tmux only
    /// ever paints inside its own pane, so anything the emulator held outside
    /// it — whatever a reflow left there when the layout last changed — was
    /// never overwritten, and sat on screen as stray characters until the pane
    /// re-attached.
    private var paneGrid: PaneGrid?

    // MARK: - Metrics

    private var font: NSFont
    private var boldFont: NSFont
    private var italicFont: NSFont
    private var cellWidth: CGFloat = 8
    private var cellHeight: CGFloat = 16
    private var baselineOffset: CGFloat = 4

    /// Insets so glyphs do not touch the pane edges.
    private let padding = TerminalMetrics.padding

    // MARK: - Selection

    /// Anchor and head in cell coordinates, while a drag is in progress or
    /// after it finishes.
    private var selection: (anchor: GridPoint, head: GridPoint)?
    /// A button the program is tracking, so its release goes to the program too.
    private var reportingButton: UInt32?

    // MARK: - Links
    //
    // ⌘-click, not shift-click as the review asked for. Shift is already the
    // selection override in `mouseDown` — the only way to copy text out of a
    // full-screen program that has grabbed the mouse — so overloading it would
    // make one gesture mean two different things depending on what happened to
    // be under the pointer, and would cost the ability to begin a selection on a
    // line containing a URL. In agent output that is most lines.
    //
    // ⌘-click is what Terminal.app and iTerm2 use, so it is a gesture people
    // already have, and it was free.

    /// The link under the pointer while ⌘ is held, and where it sits.
    private var hoveredLink: (url: String, span: FarCoolerVtUrlSpan)?
    /// Recreated on every layout, so the area always covers the current bounds.
    private var linkTracking: NSTrackingArea?
    /// Whether this view is the one currently showing the pointing hand, so it
    /// only ever restores a cursor it changed itself.
    private var showingLinkCursor = false

    /// A background to paint instead of the theme in force.
    ///
    /// For the theme editor's preview, which renders a theme that is being
    /// EDITED and has not been chosen — so the chrome has to follow the draft
    /// rather than `Themes.shared.current`, which is what every real pane
    /// correctly follows. Nil everywhere else.
    private var previewBackground: UInt32?

    /// Paint this background rather than the theme's. See `previewBackground`.
    func overrideBackground(_ packed: UInt32) {
        previewBackground = packed
        layer?.backgroundColor = Palette.cgColor(packed)
        needsDisplay = true
    }

    struct GridPoint: Equatable, Comparable {
        var row: Int
        var column: Int
        static func < (a: GridPoint, b: GridPoint) -> Bool {
            (a.row, a.column) < (b.row, b.column)
        }
    }

    init() {
        // From preferences, not a constant. A monospaced face is something
        // people have long-held preferences about, and one that renders badly
        // on your display makes the whole app unpleasant whatever it does.
        font = Preferences.shared.terminalFont()
        boldFont = Preferences.shared.terminalFont(weight: .bold)
        italicFont = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask)
        core = VTCore(columns: 80, rows: 24)
        super.init(frame: .zero)
        measure()
        wantsLayer = true
        // Dropping a file on a pane is the same intent as pasting one, and on
        // a Mac it is the more natural of the two. Any file: the thing being
        // discussed is as often a PDF or a log as a screenshot.
        registerForDraggedTypes([.fileURL, .png, .tiff])
        // Applied once up front as well as on every later change: the observer
        // below fires on CHANGES, and a view built while a theme is already
        // chosen has had no change to hear about.
        applyTheme()
        themeObserver = Themes.shared.$revision
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in
                guard let self else { return }
                self.applyTheme()
                self.needsDisplay = true
            }
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    private func measure() {
        let cell = TerminalMetrics.cell(font)
        cellWidth = cell.width
        cellHeight = cell.height
        baselineOffset = -font.descender + font.leading / 2
    }

    // MARK: - Lifecycle

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // The link retains its target, so this view cannot be deallocated while
        // one is running. Leaving the window is therefore the only moment the
        // cycle can be broken — and the only moment it needs to be.
        displayLink?.invalidate()
        guard window != nil else {
            displayLink = nil
            return
        }
        // Vsync-driven rather than a timer: a fixed timer either lags behind
        // the display or wakes up for nothing. The tick itself is cheap — it
        // compares one integer and usually does nothing at all.
        let link = displayLink(target: self, selector: #selector(tick))
        link.add(to: .main, forMode: .common)
        displayLink = link
        // Deliberately does NOT claim the keyboard.
        //
        // It used to, and with four panes that meant whichever mounted last owned
        // every keystroke — regardless of which pane the layout said was focused,
        // and regardless of which one had the focus ring drawn round it. Moving
        // focus moved the border and nothing else, so you typed into a pane you
        // were not looking at.
        //
        // Who holds the keyboard is a property of the LAYOUT now, and the surface
        // is told. A single terminal is the same rule with one pane in the list.
    }

    @objc private func tick() {
        // The core tells us when something might have changed. An idle terminal
        // — which is most of a night — costs one comparison per frame.
        let revision = core.revision
        guard revision != lastDrawnRevision else { return }
        lastDrawnRevision = revision
        needsDisplay = true

        if core.takeBell() { NSSound.beep() }
        // OSC 52: the program handing you something. Drained on the tick beside
        // the bell and the pty replies because it arrives the same way they do —
        // as a side effect of feeding bytes — and the tick is already the one
        // place that notices the core has news.
        if let copied = core.takeClipboard() {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(copied, forType: .string)
        }
        let replies = core.takePendingWrites()
        if !replies.isEmpty { onInput?(replies) }
    }

    override func layout() {
        super.layout()
        reportGeometry()
    }

    /// The view had no tracking area at all before links existed, which is why
    /// it could not previously know where the pointer was.
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let existing = linkTracking { removeTrackingArea(existing) }
        let area = NSTrackingArea(
            rect: .zero,
            // `inVisibleRect` keeps the area sized to the view without this
            // having to run again on every divider drag.
            options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self)
        addTrackingArea(area)
        linkTracking = area
    }

    /// Report the grid again even though nothing about it has changed.
    ///
    /// `reportGeometry` deduplicates, which is right for a layout pass that
    /// runs continuously and wrong for the one caller that needs the report as
    /// a TRIGGER rather than as news. `TerminalSurface.attach` starts the byte
    /// stream from inside `onGeometry`, so that the pane is resized before its
    /// history replays; a pane re-attaching because its runner reconnected has
    /// exactly the grid it always had, so the deduplication swallowed the
    /// report and the stream was never started. The reconnection looked fixed
    /// and the pane stayed as frozen as before.
    func reannounceGeometry() {
        lastReportedGeometry = PaneGrid(columns: 0, rows: 0)
        reportGeometry()
    }

    /// Size the emulator to the pane tmux actually has.
    ///
    /// Called by whoever is reading the layout, which is the only thing that
    /// knows. Nothing calls it in the single-terminal fallback or in the theme
    /// editor's preview — neither has a tmux pane behind it — so those keep
    /// sizing themselves from their own pixels, which is correct when the view
    /// is the one asking for the size rather than being told it.
    ///
    /// Takes an optional so that a grid can be taken BACK, not only given. A
    /// view told a pane's size and then moved somewhere with no layout behind it
    /// would otherwise stay pinned to a pane it has left, with no way to say so.
    func setPaneGrid(_ grid: PaneGrid?) {
        guard let grid else {
            // Only on the way from having one to not, never on every call. This
            // runs on each SwiftUI update, and the single-terminal path passes
            // nil on all of them — re-reporting there would send the daemon a
            // resize per update of a size that has not changed.
            guard paneGrid != nil else { return }
            paneGrid = nil
            // Defeat the deduplication as well, or the measurement this view is
            // falling back to would be swallowed as one it has already reported
            // and the core would keep the departed pane's size.
            lastReportedGeometry = PaneGrid(columns: 0, rows: 0)
            reportGeometry()
            return
        }
        guard grid.columns > 0, grid.rows > 0, grid != paneGrid else { return }
        paneGrid = grid
        core.resize(columns: grid.columns, rows: grid.rows)
        needsDisplay = true
    }

    /// Tell the pane what grid would fit here, and size the emulator if nobody
    /// else is going to.
    ///
    /// The measurement is a REQUEST, not an answer: it is what this view would
    /// like to be, sent on to whoever can ask tmux for it. Only when no one is
    /// reporting the pane's real grid — see `setPaneGrid` — does it also become
    /// the size the emulator is held at.
    private func reportGeometry() {
        guard
            let fits = TileGeometry.fitting(
                bounds.size, cell: CGSize(width: cellWidth, height: cellHeight))
        else { return }
        guard fits != lastReportedGeometry else { return }
        lastReportedGeometry = fits

        if paneGrid == nil {
            core.resize(columns: fits.columns, rows: fits.rows)
        }
        needsDisplay = true
        onGeometry?(fits.columns, fits.rows)
    }

    /// The grid the emulator is actually holding.
    ///
    /// Read off the core rather than remembered, so it answers what IS rather
    /// than what was last asked for — which is the distinction the pane-grid bug
    /// lived in.
    var grid: PaneGrid {
        core.withSnapshot { PaneGrid(columns: $0.columns, rows: $0.rows) }
            ?? PaneGrid(columns: 0, rows: 0)
    }

    /// Where the grid sits, so a diagnostic can check ink against cell rows.
    struct Metrics {
        let cellWidth: CGFloat
        let cellHeight: CGFloat
        let paddingTop: CGFloat
        let paddingLeft: CGFloat
    }

    var metrics: Metrics {
        Metrics(
            cellWidth: cellWidth, cellHeight: cellHeight,
            paddingTop: padding.top, paddingLeft: padding.left)
    }

    /// Re-read the font and re-measure.
    ///
    /// Changing the font changes the cell size, which changes how many columns
    /// fit, which the pane has to be told about — so this ends in the same
    /// resize path a window drag uses rather than a second one that could
    /// disagree with it.
    func applyPreferences() {
        font = Preferences.shared.terminalFont()
        boldFont = Preferences.shared.terminalFont(weight: .bold)
        italicFont = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask)
        applyTheme()
        measure()
        lastReportedGeometry = PaneGrid(columns: 0, rows: 0)
        reportGeometry()
        needsDisplay = true
    }

    /// Hand the palette to the core.
    ///
    /// Chrome alone is not enough: every CELL's colour is resolved inside the
    /// emulator — deliberately, so three renderers cannot drift — so a theme
    /// the core has not been told about would repaint the background and leave
    /// every character in the old colours.
    func applyTheme() {
        core.setPalette(Themes.shared.current.packed)
        layer?.backgroundColor = Palette.background.cgColor
    }

    /// Replace the core, for when the view is pointed at a different terminal.
    func reset(columns: Int, rows: Int) {
        core = VTCore(columns: columns, rows: rows)
        // A fresh core starts on the VT crate's own default palette, which is
        // not the theme in force. Without this, pointing a view at a different
        // terminal repainted its chrome correctly and left every character in
        // the wrong colours.
        core.setPalette(Themes.shared.current.packed)
        selection = nil
        lastDrawnRevision = .max
        // The new core is the size this call asked for, so both memories of the
        // old one are claims about a terminal that is gone. Left in place, the
        // pane grid would suppress the next `setPaneGrid` for the same numbers
        // and the fresh core would never be told its size; the reported geometry
        // would suppress the next `reportGeometry`, and with it the `onGeometry`
        // that opens the byte stream — so a re-pointed view would size itself
        // correctly and then sit there receiving nothing.
        paneGrid = nil
        lastReportedGeometry = PaneGrid(columns: 0, rows: 0)
        needsDisplay = true
    }

    func feed(_ bytes: [UInt8]) {
        core.feed(bytes)
    }

    // MARK: - Focus
    //
    // SwiftUI's split view parks first responder in the sidebar, so without
    // claiming it the terminal renders perfectly and ignores every keystroke.

    func claimKeyboard() {
        DispatchQueue.main.async { [weak self] in
            guard let self, let window = self.window, window.firstResponder !== self else { return }
            window.makeFirstResponder(self)
        }
    }

    override func becomeFirstResponder() -> Bool {
        needsDisplay = true
        return super.becomeFirstResponder()
    }

    override func resignFirstResponder() -> Bool {
        needsDisplay = true
        return super.resignFirstResponder()
    }

    // MARK: - Drawing

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        context.setFillColor(previewBackground.map(Palette.cgColor) ?? Palette.background.cgColor)
        context.fill(bounds)

        core.withSnapshot { snapshot in
            drawBackgrounds(snapshot, in: context)
            drawSelection(snapshot, in: context)
            drawGlyphs(snapshot, in: context)
            drawLinkUnderline(snapshot, in: context)
            drawCursor(snapshot, in: context)
        }
    }

    /// Underline the ⌘-hovered link, on every row it covers.
    ///
    /// Drawn here rather than inside `drawGlyphs`, which flips into text space:
    /// this measures in the same top-down space `origin` reports, like the
    /// background fills do.
    private func drawLinkUnderline(_ snapshot: VTSnapshot, in context: CGContext) {
        guard let link = hoveredLink, snapshot.columns > 0 else { return }
        // Clamped to the grid that exists right now. The span was measured
        // against a snapshot taken a moment ago, and a pane can be resized
        // between the two — the same reflow hazard `selectionSpan` documents.
        let first = max(0, Int(link.span.start_row))
        let last = min(snapshot.rows - 1, Int(link.span.end_row))
        guard first <= last else { return }

        context.setLineWidth(1)
        for row in first...last {
            let from = row == first ? min(Int(link.span.start_column), snapshot.columns - 1) : 0
            let to = row == last
                ? min(Int(link.span.end_column), snapshot.columns - 1) : snapshot.columns - 1
            guard to >= from else { continue }

            // The link's own color, so it underlines in whatever the program
            // painted it rather than in a color no theme chose.
            context.setStrokeColor(Palette.cgColor(effectiveForeground(snapshot[row, from])))
            let start = origin(row: row, column: from)
            let y = start.y + cellHeight - 1.5
            context.move(to: CGPoint(x: start.x, y: y))
            context.addLine(to: CGPoint(x: origin(row: row, column: to).x + cellWidth, y: y))
            context.strokePath()
        }
    }

    private func origin(row: Int, column: Int) -> CGPoint {
        CGPoint(
            x: padding.left + CGFloat(column) * cellWidth,
            y: padding.top + CGFloat(row) * cellHeight
        )
    }

    /// Where a cell's glyph sits, in the bottom-up space text is drawn in.
    private func baselinePoint(row: Int, column: Int) -> CGPoint {
        let point = origin(row: row, column: column)
        return CGPoint(x: point.x, y: bounds.height - (point.y + cellHeight - baselineOffset))
    }

    /// Fill background runs. Batched, because a full-width bar is one rect, not
    /// two hundred.
    private func drawBackgrounds(_ snapshot: VTSnapshot, in context: CGContext) {
        let defaultBG = Palette.backgroundPacked
        for row in 0..<snapshot.rows {
            var column = 0
            while column < snapshot.columns {
                let color = effectiveBackground(snapshot[row, column])
                if color == defaultBG {
                    column += 1
                    continue
                }
                var end = column + 1
                while end < snapshot.columns, effectiveBackground(snapshot[row, end]) == color {
                    end += 1
                }
                let start = origin(row: row, column: column)
                context.setFillColor(Palette.cgColor(color))
                context.fill(
                    CGRect(
                        x: start.x, y: start.y,
                        width: CGFloat(end - column) * cellWidth, height: cellHeight))
                column = end
            }
        }
    }

    private func effectiveBackground(_ cell: FarCoolerVtCell) -> UInt32 {
        cell.isInverse ? cell.fg : cell.bg
    }

    private func effectiveForeground(_ cell: FarCoolerVtCell) -> UInt32 {
        cell.isInverse ? cell.bg : cell.fg
    }

    /// Draw text.
    ///
    /// Glyphs are positioned per cell rather than laid out as a line: a text
    /// layout engine applies kerning and ligatures, which look lovely and put
    /// every character in the wrong column. A terminal is a grid; each glyph
    /// sits at its own cell origin.
    private func drawGlyphs(_ snapshot: VTSnapshot, in context: CGContext) {
        context.saveGState()
        defer { context.restoreGState() }
        // Glyph positions passed to CTFontDrawGlyphs are in TEXT space, so a
        // flip expressed in the text matrix negates them and every glyph lands
        // off the top of the view. Flip the coordinate system instead and hand
        // over bottom-up positions; the text matrix stays identity.
        context.textMatrix = .identity
        context.translateBy(x: 0, y: bounds.height)
        context.scaleBy(x: 1, y: -1)

        var glyphs: [CGGlyph] = []
        var positions: [CGPoint] = []
        var fallbacks: [(Character, CGPoint, UInt32, NSFont)] = []

        for row in 0..<snapshot.rows {
            var column = 0
            while column < snapshot.columns {
                let cell = snapshot[row, column]
                let step = cell.isWide ? 2 : 1
                guard let character = cell.character else {
                    column += step
                    continue
                }

                // A run is a stretch of cells sharing color and face, which is
                // what a whole word or a whole log line usually is.
                let color = effectiveForeground(cell)
                let face = self.face(for: cell)
                glyphs.removeAll(keepingCapacity: true)
                positions.removeAll(keepingCapacity: true)

                var end = column
                var underlineStart = cell.isUnderlined ? column : -1
                while end < snapshot.columns {
                    let next = snapshot[row, end]
                    guard effectiveForeground(next) == color, self.face(for: next) === face,
                        next.isUnderlined == cell.isUnderlined
                    else { break }
                    if let ch = next.character {
                        let baseline = baselinePoint(row: row, column: end)
                        if let glyph = glyph(for: ch, in: face) {
                            glyphs.append(glyph)
                            positions.append(baseline)
                        } else {
                            // Emoji and anything outside the mono font. Rare, so
                            // it can afford an individual draw.
                            fallbacks.append((ch, baseline, color, face))
                        }
                    }
                    end += next.isWide ? 2 : 1
                }

                if !glyphs.isEmpty {
                    context.setFillColor(Palette.cgColor(color))
                    CTFontDrawGlyphs(face, glyphs, positions, glyphs.count, context)
                }
                if underlineStart >= 0 {
                    context.setStrokeColor(Palette.cgColor(color))
                    context.setLineWidth(1)
                    let y = bounds.height - (origin(row: row, column: 0).y + cellHeight - 1.5)
                    context.move(to: CGPoint(x: origin(row: row, column: underlineStart).x, y: y))
                    context.addLine(to: CGPoint(x: origin(row: row, column: end).x, y: y))
                    context.strokePath()
                    underlineStart = -1
                }
                column = max(end, column + step)
            }
        }

        for (character, point, color, face) in fallbacks {
            draw(character: character, at: point, color: color, preferring: face, in: context)
        }
    }

    private func face(for cell: FarCoolerVtCell) -> NSFont {
        if cell.isBold { return boldFont }
        if cell.isItalic { return italicFont }
        return font
    }

    private func glyph(for character: Character, in face: NSFont) -> CGGlyph? {
        let utf16 = Array(String(character).utf16)
        // A surrogate pair is one glyph in two units; the simple path handles
        // the BMP, and the fallback handles the rest.
        guard utf16.count == 1 else { return nil }
        var glyph = CGGlyph()
        var unit = utf16[0]
        guard CTFontGetGlyphsForCharacters(face, &unit, &glyph, 1), glyph != 0 else { return nil }
        return glyph
    }

    /// Draw one character the mono font cannot, letting CoreText pick a font
    /// that can. This is how emoji in agent output render at all.
    private func draw(
        character: Character, at point: CGPoint, color: UInt32, preferring face: NSFont,
        in context: CGContext
    ) {
        let string = String(character) as NSString
        let attributed = NSAttributedString(
            string: string as String,
            attributes: [
                .font: face,
                .foregroundColor: NSColor(cgColor: Palette.cgColor(color)) ?? .white,
            ])
        let line = CTLineCreateWithAttributedString(attributed)
        context.textPosition = point
        CTLineDraw(line, context)
    }

    private func drawCursor(_ snapshot: VTSnapshot, in context: CGContext) {
        guard snapshot.cursorVisible, snapshot.cursorRow < snapshot.rows,
            snapshot.cursorColumn < snapshot.columns
        else { return }

        let point = origin(row: snapshot.cursorRow, column: snapshot.cursorColumn)
        let cell = snapshot[snapshot.cursorRow, snapshot.cursorColumn]
        let rect = CGRect(
            x: point.x, y: point.y,
            width: cell.isWide ? cellWidth * 2 : cellWidth, height: cellHeight)

        let focused = window?.firstResponder === self
        if !focused {
            // Hollow when unfocused: the difference between "typing goes here"
            // and "this terminal is just showing you something".
            context.setStrokeColor(Palette.cgColor(effectiveForeground(cell)).copy(alpha: 0.5)!)
            context.setLineWidth(1)
            context.stroke(rect.insetBy(dx: 0.5, dy: 0.5))
            return
        }

        context.setFillColor(Palette.cursor.cgColor)
        context.fill(rect)
        if let character = cell.character {
            context.saveGState()
            context.textMatrix = .identity
            context.translateBy(x: 0, y: bounds.height)
            context.scaleBy(x: 1, y: -1)
            draw(
                character: character,
                at: baselinePoint(row: snapshot.cursorRow, column: snapshot.cursorColumn),
                color: Palette.backgroundPacked,
                preferring: face(for: cell), in: context)
            context.restoreGState()
        }
    }

    // MARK: - Keyboard

    override func keyDown(with event: NSEvent) {
        // The tiling prefix gets first refusal, because it has to: with a
        // terminal focused there is nothing else in the responder chain that
        // sees ⌃B before the emulator encodes it and sends it to the program.
        //
        // `handled` covers arming the prefix and the key that follows it.
        // Everything else falls through untouched, including ⌃B ⌃B — which is how
        // a literal ⌃B still reaches a tmux running inside this pane.
        if MainActor.assumeIsolated({ PrefixMode.shared.handle(event) }) == .handled {
            return
        }

        // Reading and typing are different intents: a keystroke means "act on
        // the live screen", so it always returns there first.
        core.scrollToBottomIfScrolled()
        selection = nil

        var modifiers: VTModifiers = []
        if event.modifierFlags.contains(.shift) { modifiers.insert(.shift) }
        if event.modifierFlags.contains(.control) { modifiers.insert(.control) }
        if event.modifierFlags.contains(.option) { modifiers.insert(.alt) }

        // Command is a macOS chord, not a terminal one. Let the menu have it.
        if event.modifierFlags.contains(.command) {
            super.keyDown(with: event)
            return
        }

        if let special = Self.specialKey(for: event) {
            send(core.encode(key: special, modifiers: modifiers))
            return
        }

        // Ignoring modifiers gives the unshifted key, which is what Ctrl and
        // Alt chords are named after: Ctrl-Shift-C is still Ctrl-C's key.
        let source =
            modifiers.contains(.control) || modifiers.contains(.alt)
            ? event.charactersIgnoringModifiers : event.characters
        guard let source, !source.isEmpty else { return }

        var bytes: [UInt8] = []
        for scalar in source.unicodeScalars {
            bytes.append(contentsOf: core.encode(scalar: scalar, modifiers: modifiers))
        }
        send(bytes)
    }

    private static func specialKey(for event: NSEvent) -> UInt32? {
        // Function keys arrive as private-use scalars, not as key codes, which
        // is stable across keyboard layouts.
        if let scalar = event.charactersIgnoringModifiers?.unicodeScalars.first {
            switch Int(scalar.value) {
            case NSUpArrowFunctionKey: return UInt32(FARCOOLER_VT_KEY_UP)
            case NSDownArrowFunctionKey: return UInt32(FARCOOLER_VT_KEY_DOWN)
            case NSLeftArrowFunctionKey: return UInt32(FARCOOLER_VT_KEY_LEFT)
            case NSRightArrowFunctionKey: return UInt32(FARCOOLER_VT_KEY_RIGHT)
            case NSHomeFunctionKey: return UInt32(FARCOOLER_VT_KEY_HOME)
            case NSEndFunctionKey: return UInt32(FARCOOLER_VT_KEY_END)
            case NSPageUpFunctionKey: return UInt32(FARCOOLER_VT_KEY_PAGE_UP)
            case NSPageDownFunctionKey: return UInt32(FARCOOLER_VT_KEY_PAGE_DOWN)
            case NSInsertFunctionKey: return UInt32(FARCOOLER_VT_KEY_INSERT)
            case NSDeleteFunctionKey: return UInt32(FARCOOLER_VT_KEY_DELETE)
            case NSF1FunctionKey...NSF12FunctionKey:
                return UInt32(FARCOOLER_VT_KEY_F1) + UInt32(Int(scalar.value) - NSF1FunctionKey)
            default: break
            }
        }
        switch event.keyCode {
        case 36, 76: return UInt32(FARCOOLER_VT_KEY_ENTER)  // Return, keypad Enter
        case 48: return UInt32(FARCOOLER_VT_KEY_TAB)
        case 51: return UInt32(FARCOOLER_VT_KEY_BACKSPACE)
        case 53: return UInt32(FARCOOLER_VT_KEY_ESCAPE)
        case 117: return UInt32(FARCOOLER_VT_KEY_DELETE)  // forward delete
        default: return nil
        }
    }

    private func send(_ bytes: [UInt8]) {
        guard !bytes.isEmpty else { return }
        onInput?(bytes)
    }

    // MARK: - Menu commands

    @objc func copy(_ sender: Any?) {
        guard let text = selectedText(), !text.isEmpty else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    @objc func paste(_ sender: Any?) {
        // Text wins whenever there is any, so this changes nothing about
        // pasting a command, a path, or anything else people already do. Only a
        // pasteboard with no text at all — a screenshot, a copy out of Preview,
        // a file dragged from the Finder — becomes an image paste.
        if let text = NSPasteboard.general.string(forType: .string) {
            typePaste(text)
            return
        }
        if let pasted = Self.image(on: NSPasteboard.general) {
            onPasteImage?(pasted)
        }
    }

    /// Type text into the pane as a paste, bracketed if the program asked.
    func typePaste(_ text: String) {
        core.scrollToBottomIfScrolled()
        send(core.encode(paste: text))
    }

    /// Something worth sending on a pasteboard, as a file when it has one.
    ///
    /// The file is preferred over the data: it is the same bytes without a
    /// re-encode, it carries a name worth putting in a prompt, and against a
    /// local daemon it means nothing has to be copied anywhere at all.
    ///
    /// Any file type. What the agent can do with a `.parquet` is the agent's
    /// business; refusing to carry one is this app deciding for it.
    static func image(on pasteboard: NSPasteboard) -> PastedImage? {
        if let urls = pasteboard.readObjects(forClasses: [NSURL.self]) as? [URL],
            let url = urls.first(where: { $0.isFileURL && !$0.hasDirectoryPath })
        {
            return .file(url)
        }
        // `png` before `tiff`: AppKit offers TIFF for everything, and a
        // screenshot re-encoded as TIFF is several times the bytes for a
        // picture of text that was already lossless.
        if let png = pasteboard.data(forType: .png) {
            return .data(png, mime: "image/png")
        }
        if let tiff = pasteboard.data(forType: .tiff), let image = NSImage(data: tiff),
            let rep = image.tiffRepresentation.flatMap({ NSBitmapImageRep(data: $0) }),
            let png = rep.representation(using: .png, properties: [:])
        {
            return .data(png, mime: "image/png")
        }
        return nil
    }

    // MARK: - Drag and drop

    override func draggingEntered(_ sender: any NSDraggingInfo) -> NSDragOperation {
        Self.image(on: sender.draggingPasteboard) != nil ? .copy : []
    }

    override func performDragOperation(_ sender: any NSDraggingInfo) -> Bool {
        guard let pasted = Self.image(on: sender.draggingPasteboard) else { return false }
        onPasteImage?(pasted)
        return true
    }

    @objc override func selectAll(_ sender: Any?) {
        core.withSnapshot { snapshot in
            selection = (
                GridPoint(row: 0, column: 0),
                GridPoint(row: snapshot.rows - 1, column: snapshot.columns - 1)
            )
        }
        needsDisplay = true
    }

    func validateUserInterfaceItem(_ item: any NSValidatedUserInterfaceItem) -> Bool {
        switch item.action {
        case #selector(copy(_:)): return selection != nil
        case #selector(paste(_:)), #selector(selectAll(_:)): return true
        default: return true
        }
    }

    // MARK: - Mouse

    /// Which cell the pointer is over, clamped to cells that exist.
    ///
    /// Clamped at BOTH ends, which it did not used to need to be. The view was
    /// once exactly as big as its grid, so dividing its pixels by a cell could
    /// not name a cell off the end of it. Now the grid is tmux's and the view is
    /// whatever the layout gave it, and the two are a cell or three apart —
    /// which leaves a thin band of background below and to the right of the last
    /// row that is inside the view and outside the terminal.
    ///
    /// Unclamped, a click in that band reported row 58 of a 54-row screen to
    /// whatever is running. `encode_mouse` states the number it is given, so a
    /// full-screen program — vim, htop, lazygit — would have acted on a
    /// coordinate off its own screen.
    private func cell(for event: NSEvent) -> GridPoint {
        let point = convert(event.locationInWindow, from: nil)
        let column = Int(((point.x - padding.left) / cellWidth).rounded(.down))
        let row = Int(((point.y - padding.top) / cellHeight).rounded(.down))
        let grid = self.grid
        return GridPoint(
            row: min(max(0, row), max(0, grid.rows - 1)),
            column: min(max(0, column), max(0, grid.columns - 1)))
    }

    /// `cell(for:)`, reachable from a test. The clamp it applies is the whole
    /// point of it and is otherwise only observable by running a program that
    /// tracks the mouse and watching it act on a row it does not have.
    func cellForTesting(_ event: NSEvent) -> GridPoint { cell(for: event) }

    private func modifiers(for event: NSEvent) -> VTModifiers {
        var m: VTModifiers = []
        if event.modifierFlags.contains(.shift) { m.insert(.shift) }
        if event.modifierFlags.contains(.control) { m.insert(.control) }
        if event.modifierFlags.contains(.option) { m.insert(.alt) }
        return m
    }

    override func mouseMoved(with event: NSEvent) {
        updateHoveredLink(for: event)
    }

    override func mouseExited(with event: NSEvent) {
        setHoveredLink(nil)
    }

    override func flagsChanged(with event: NSEvent) {
        // ⌘ pressed or released without the pointer moving still has to
        // underline or un-underline whatever is under it, so the modifier is a
        // trigger in its own right rather than only a condition on a move.
        super.flagsChanged(with: event)
        updateHoveredLink(for: event)
    }

    /// The link under the pointer, or nil when ⌘ is not held.
    private func updateHoveredLink(for event: NSEvent) {
        guard event.modifierFlags.contains(.command) else {
            setHoveredLink(nil)
            return
        }
        let point = cell(for: event)
        setHoveredLink(core.url(atRow: point.row, column: point.column))
    }

    private func setHoveredLink(_ link: (url: String, span: FarCoolerVtUrlSpan)?) {
        let changed = link?.url != hoveredLink?.url
        hoveredLink = link

        // Only the pointing hand is this view's to set, and it is only restored
        // if this view was the one that changed it. The alternative — picking a
        // cursor for the no-link case — would mean inventing a base cursor for a
        // view that never had one, and overriding it everywhere else.
        if link != nil {
            NSCursor.pointingHand.set()
            showingLinkCursor = true
        } else if showingLinkCursor {
            NSCursor.arrow.set()
            showingLinkCursor = false
        }

        // Redraw only when the underline would actually move. This runs on every
        // mouse move, and a frame per pixel of pointer travel would be spent on
        // a grid that has not changed.
        if changed { needsDisplay = true }
    }

    override func mouseDown(with event: NSEvent) {
        // Before anything else, so a program tracking the mouse never sees this
        // click and `claimKeyboard` does not steal focus for what is really a
        // click on a link. The core decides what a URL is and which schemes may
        // be opened — terminal output is not trusted input, and an agent prints
        // whatever it read.
        if event.modifierFlags.contains(.command) {
            let target = cell(for: event)
            if let link = core.url(atRow: target.row, column: target.column),
                let url = URL(string: link.url)
            {
                NSWorkspace.shared.open(url)
                return
            }
        }

        claimKeyboard()
        let point = cell(for: event)

        // Shift is the universal override for "I want to select, not click" —
        // without it there is no way to copy text out of a full-screen program.
        if !event.modifierFlags.contains(.shift),
            let bytes = core.encode(
                mouse: UInt32(FARCOOLER_VT_MOUSE_LEFT), action: UInt32(FARCOOLER_VT_MOUSE_PRESS),
                column: point.column, row: point.row, modifiers: modifiers(for: event))
        {
            reportingButton = UInt32(FARCOOLER_VT_MOUSE_LEFT)
            send(bytes)
            return
        }

        reportingButton = nil
        selection = (point, point)
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        let point = cell(for: event)
        if let button = reportingButton {
            if let bytes = core.encode(
                mouse: button, action: UInt32(FARCOOLER_VT_MOUSE_MOVE),
                column: point.column, row: point.row, modifiers: modifiers(for: event))
            {
                send(bytes)
            }
            return
        }
        guard let current = selection else { return }
        selection = (current.anchor, point)
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        let point = cell(for: event)
        if let button = reportingButton {
            reportingButton = nil
            if let bytes = core.encode(
                mouse: button, action: UInt32(FARCOOLER_VT_MOUSE_RELEASE),
                column: point.column, row: point.row, modifiers: modifiers(for: event))
            {
                send(bytes)
            }
            return
        }
        // A click with no drag is a click, not an empty selection.
        if let current = selection, current.anchor == current.head {
            selection = nil
            needsDisplay = true
        }
    }

    override func scrollWheel(with event: NSEvent) {
        // Ask the core first: a full-screen program may want the wheel, in
        // which case it becomes a mouse report or arrow keys, and scrolling our
        // own view would be wrong.
        let ticks = wheelTicks(event)
        guard ticks != 0 else { return }
        let point = cell(for: event)
        let button =
            ticks > 0
            ? UInt32(FARCOOLER_VT_MOUSE_WHEEL_UP) : UInt32(FARCOOLER_VT_MOUSE_WHEEL_DOWN)

        var bytes: [UInt8] = []
        for _ in 0..<abs(ticks) {
            guard
                let chunk = core.encode(
                    mouse: button, action: UInt32(FARCOOLER_VT_MOUSE_PRESS),
                    column: point.column, row: point.row, modifiers: modifiers(for: event))
            else {
                // The program does not want it, so it is ours: scroll history.
                core.scroll(lines: Int32(ticks))
                needsDisplay = true
                return
            }
            bytes.append(contentsOf: chunk)
        }
        send(bytes)
    }

    /// How many lines one detent of a wheel is worth.
    ///
    /// Three, which is the number every platform settled on decades ago and
    /// every terminal inherited. It is not arbitrary here either: this is the
    /// only place in Far Cooler that can apply it, because it is the only place
    /// that knows a notch from a finger.
    private static let linesPerNotch = 3

    /// Convert a scroll gesture to whole lines.
    ///
    /// A trackpad reports continuous pixel deltas, and that is direct
    /// manipulation: the content follows the finger, one line per cell height
    /// of travel. Accumulating the remainder keeps a slow drag from being
    /// rounded away to nothing.
    ///
    /// A wheel reports notches, which are not a distance at all — macOS hands
    /// over a count of detents and leaves the distance to the application. This
    /// used to return that count unmultiplied, so one notch scrolled exactly one
    /// line, wherever the tick ended up: a mouse report to Claude Code, which
    /// scrolls its transcript by one line per report; an arrow key to `less`;
    /// one line of our own scrollback for Codex. Every one of them was correct
    /// about the tick and a third of the way there.
    private var scrollRemainder: CGFloat = 0

    private func wheelTicks(_ event: NSEvent) -> Int {
        if event.hasPreciseScrollingDeltas {
            scrollRemainder += event.scrollingDeltaY
            let lines = (scrollRemainder / cellHeight).rounded(.towardZero)
            scrollRemainder -= lines * cellHeight
            return Int(lines)
        }
        scrollRemainder = 0
        return Int(event.scrollingDeltaY.rounded()) * Self.linesPerNotch
    }

    // MARK: - Selection

    /// The selection, ordered and clipped to the grid that exists right now.
    ///
    /// `nil` when it no longer touches the screen at all, which is a state that
    /// happens constantly: the grid reflows under a live selection every time a
    /// pane is resized, a divider is dragged, or the window changes size.
    ///
    /// This used to be `start.row...min(end.row, snapshot.rows - 1)`, which is
    /// correct only while the selection is inside the grid. Shrink a pane below
    /// the selection's start row — drag a divider up, say — and the clamped upper
    /// bound falls below the lower one, which is not an empty range in Swift but a
    /// trap. It crashed the app from `draw`, so the terminal took the process down
    /// while merely redrawing itself.
    private func selectionSpan(
        in snapshot: VTSnapshot
    ) -> (start: GridPoint, end: GridPoint, rows: ClosedRange<Int>)? {
        guard let selection, snapshot.rows > 0, snapshot.columns > 0 else { return nil }
        let (start, end) =
            selection.anchor <= selection.head
            ? (selection.anchor, selection.head) : (selection.head, selection.anchor)

        let first = max(0, start.row)
        let last = min(end.row, snapshot.rows - 1)
        guard first <= last else { return nil }
        return (start, end, first...last)
    }

    private func drawSelection(_ snapshot: VTSnapshot, in context: CGContext) {
        guard let (start, end, rows) = selectionSpan(in: snapshot) else { return }

        context.setFillColor(Palette.selection.cgColor)
        for row in rows {
            let first = max(0, row == start.row ? start.column : 0)
            let last = min(
                snapshot.columns - 1, row == end.row ? end.column : snapshot.columns - 1)
            guard last >= first else { continue }
            let point = origin(row: row, column: first)
            context.fill(
                CGRect(
                    x: point.x, y: point.y,
                    width: CGFloat(last - first + 1) * cellWidth, height: cellHeight))
        }
    }

    private func selectedText() -> String? {
        guard selection != nil else { return nil }
        return core.withSnapshot { snapshot -> String in
            guard let (start, end, rows) = self.selectionSpan(in: snapshot) else { return "" }
            var lines: [String] = []
            for row in rows {
                let first = max(0, row == start.row ? start.column : 0)
                let last = min(
                    snapshot.columns - 1, row == end.row ? end.column : snapshot.columns - 1)
                guard last >= first else { continue }
                var line = ""
                for column in first...last {
                    let cell = snapshot[row, column]
                    // A spacer column belongs to the wide character before it.
                    if column > first, snapshot[row, column - 1].isWide { continue }
                    line.append(cell.character ?? " ")
                }
                // Trailing blanks are padding, not content: copying them turns
                // a pasted command into one with a hundred trailing spaces.
                lines.append(String(line.reversed().drop { $0 == " " }.reversed()))
            }
            return lines.joined(separator: "\n")
        }
    }
}

/// Colors the renderer owns. Cell colors come from the core already resolved;
/// these are the chrome around them.
///
/// Read from the theme in force rather than fixed, and read on each access
/// rather than captured: these were `static let`s, which is what made the
/// palette unchangeable without relaunching. The cost is a dictionary lookup
/// per draw call against a value that is already in memory.
@MainActor
enum Palette {
    private static var theme: Theme { Themes.shared.current }

    static var backgroundPacked: UInt32 { theme.background }
    static var background: NSColor { theme.backgroundColor }
    /// The theme's body text, for chrome drawn ON `background`.
    ///
    /// Cells never need this — the core resolves each one's own color — but
    /// anything this app paints over a terminal ground does, and the two have
    /// to move together. `ScreenPreview` is the case that proves it: it had a
    /// hardcoded white, which was correct while this enum was `static let`s of
    /// one dark palette and became invisible the moment `background` started
    /// following the theme. Three shipped themes are `#FFFFFF`.
    static var foreground: NSColor { theme.foregroundColor }
    static var cursor: NSColor { theme.cursorColor }
    static var selection: NSColor { theme.selectionColor }

    static func cgColor(_ packed: UInt32) -> CGColor {
        CGColor(
            srgbRed: CGFloat((packed >> 16) & 0xFF) / 255,
            green: CGFloat((packed >> 8) & 0xFF) / 255,
            blue: CGFloat(packed & 0xFF) / 255,
            alpha: 1)
    }
}
