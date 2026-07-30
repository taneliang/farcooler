import AppKit
import COvernightVT
import CoreText

/// Draws the terminal and routes input to it.
///
/// The whole of this view's job is pixels and events. It does not parse an
/// escape sequence, does not know what an arrow key means, and does not decide
/// what colour `\e[31m` is — the Rust core answers all of that, which is what
/// lets the same answers serve iOS and Android. What remains here is genuinely
/// platform work: fonts, glyph rasterisation, NSEvent, the pasteboard.
@MainActor
final class TerminalRenderView: NSView, NSUserInterfaceValidations {
    /// Encoded bytes leaving the terminal, ready for the pane.
    var onInput: (([UInt8]) -> Void)?
    /// The grid this view can show, which is what the pane must be resized to.
    var onGeometry: ((Int, Int) -> Void)?

    private(set) var core: VTCore
    private var displayLink: CADisplayLink?
    private var lastDrawnRevision: UInt64 = .max
    private var lastReportedGeometry = (columns: 0, rows: 0)

    // MARK: - Metrics

    private var font: NSFont
    private var boldFont: NSFont
    private var italicFont: NSFont
    private var cellWidth: CGFloat = 8
    private var cellHeight: CGFloat = 16
    private var baselineOffset: CGFloat = 4

    /// Insets so glyphs do not touch the pane edges.
    private let padding = NSEdgeInsets(top: 8, left: 10, bottom: 8, right: 10)

    // MARK: - Selection

    /// Anchor and head in cell coordinates, while a drag is in progress or
    /// after it finishes.
    private var selection: (anchor: GridPoint, head: GridPoint)?
    /// A button the program is tracking, so its release goes to the program too.
    private var reportingButton: UInt32?

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
        layer?.backgroundColor = Palette.background.cgColor
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    private func measure() {
        // Every cell is the same box, so one advance defines the grid. Measured
        // rather than assumed, because the user's font size changes it.
        var glyph = CGGlyph()
        var ch: UniChar = 0x4D  // "M"
        CTFontGetGlyphsForCharacters(font, &ch, &glyph, 1)
        var advance = CGSize.zero
        CTFontGetAdvancesForGlyphs(font, .horizontal, &glyph, &advance, 1)

        cellWidth = advance.width
        cellHeight = (font.ascender - font.descender + font.leading).rounded(.up)
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
        claimKeyboard()
    }

    @objc private func tick() {
        // The core tells us when something might have changed. An idle terminal
        // — which is most of a night — costs one comparison per frame.
        let revision = core.revision
        guard revision != lastDrawnRevision else { return }
        lastDrawnRevision = revision
        needsDisplay = true

        if core.takeBell() { NSSound.beep() }
        let replies = core.takePendingWrites()
        if !replies.isEmpty { onInput?(replies) }
    }

    override func layout() {
        super.layout()
        reportGeometry()
    }

    /// Tell the core and the pane what grid actually fits.
    private func reportGeometry() {
        let usableWidth = bounds.width - padding.left - padding.right
        let usableHeight = bounds.height - padding.top - padding.bottom
        guard usableWidth > 0, usableHeight > 0, cellWidth > 0, cellHeight > 0 else { return }

        let columns = max(20, Int(usableWidth / cellWidth))
        let rows = max(5, Int(usableHeight / cellHeight))
        guard (columns, rows) != lastReportedGeometry else { return }
        lastReportedGeometry = (columns, rows)

        core.resize(columns: columns, rows: rows)
        needsDisplay = true
        onGeometry?(columns, rows)
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
        measure()
        lastReportedGeometry = (0, 0)
        reportGeometry()
        needsDisplay = true
    }

    /// Replace the core, for when the view is pointed at a different terminal.
    func reset(columns: Int, rows: Int) {
        core = VTCore(columns: columns, rows: rows)
        selection = nil
        lastDrawnRevision = .max
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
        context.setFillColor(Palette.background.cgColor)
        context.fill(bounds)

        core.withSnapshot { snapshot in
            drawBackgrounds(snapshot, in: context)
            drawSelection(snapshot, in: context)
            drawGlyphs(snapshot, in: context)
            drawCursor(snapshot, in: context)
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
                let colour = effectiveBackground(snapshot[row, column])
                if colour == defaultBG {
                    column += 1
                    continue
                }
                var end = column + 1
                while end < snapshot.columns, effectiveBackground(snapshot[row, end]) == colour {
                    end += 1
                }
                let start = origin(row: row, column: column)
                context.setFillColor(Palette.cgColor(colour))
                context.fill(
                    CGRect(
                        x: start.x, y: start.y,
                        width: CGFloat(end - column) * cellWidth, height: cellHeight))
                column = end
            }
        }
    }

    private func effectiveBackground(_ cell: OvernightVtCell) -> UInt32 {
        cell.isInverse ? cell.fg : cell.bg
    }

    private func effectiveForeground(_ cell: OvernightVtCell) -> UInt32 {
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

                // A run is a stretch of cells sharing colour and face, which is
                // what a whole word or a whole log line usually is.
                let colour = effectiveForeground(cell)
                let face = self.face(for: cell)
                glyphs.removeAll(keepingCapacity: true)
                positions.removeAll(keepingCapacity: true)

                var end = column
                var underlineStart = cell.isUnderlined ? column : -1
                while end < snapshot.columns {
                    let next = snapshot[row, end]
                    guard effectiveForeground(next) == colour, self.face(for: next) === face,
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
                            fallbacks.append((ch, baseline, colour, face))
                        }
                    }
                    end += next.isWide ? 2 : 1
                }

                if !glyphs.isEmpty {
                    context.setFillColor(Palette.cgColor(colour))
                    CTFontDrawGlyphs(face, glyphs, positions, glyphs.count, context)
                }
                if underlineStart >= 0 {
                    context.setStrokeColor(Palette.cgColor(colour))
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

        for (character, point, colour, face) in fallbacks {
            draw(character: character, at: point, colour: colour, preferring: face, in: context)
        }
    }

    private func face(for cell: OvernightVtCell) -> NSFont {
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
        character: Character, at point: CGPoint, colour: UInt32, preferring face: NSFont,
        in context: CGContext
    ) {
        let string = String(character) as NSString
        let attributed = NSAttributedString(
            string: string as String,
            attributes: [
                .font: face,
                .foregroundColor: NSColor(cgColor: Palette.cgColor(colour)) ?? .white,
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
                colour: Palette.backgroundPacked,
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
            case NSUpArrowFunctionKey: return UInt32(OVERNIGHT_VT_KEY_UP)
            case NSDownArrowFunctionKey: return UInt32(OVERNIGHT_VT_KEY_DOWN)
            case NSLeftArrowFunctionKey: return UInt32(OVERNIGHT_VT_KEY_LEFT)
            case NSRightArrowFunctionKey: return UInt32(OVERNIGHT_VT_KEY_RIGHT)
            case NSHomeFunctionKey: return UInt32(OVERNIGHT_VT_KEY_HOME)
            case NSEndFunctionKey: return UInt32(OVERNIGHT_VT_KEY_END)
            case NSPageUpFunctionKey: return UInt32(OVERNIGHT_VT_KEY_PAGE_UP)
            case NSPageDownFunctionKey: return UInt32(OVERNIGHT_VT_KEY_PAGE_DOWN)
            case NSInsertFunctionKey: return UInt32(OVERNIGHT_VT_KEY_INSERT)
            case NSDeleteFunctionKey: return UInt32(OVERNIGHT_VT_KEY_DELETE)
            case NSF1FunctionKey...NSF12FunctionKey:
                return UInt32(OVERNIGHT_VT_KEY_F1) + UInt32(Int(scalar.value) - NSF1FunctionKey)
            default: break
            }
        }
        switch event.keyCode {
        case 36, 76: return UInt32(OVERNIGHT_VT_KEY_ENTER)  // Return, keypad Enter
        case 48: return UInt32(OVERNIGHT_VT_KEY_TAB)
        case 51: return UInt32(OVERNIGHT_VT_KEY_BACKSPACE)
        case 53: return UInt32(OVERNIGHT_VT_KEY_ESCAPE)
        case 117: return UInt32(OVERNIGHT_VT_KEY_DELETE)  // forward delete
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
        guard let text = NSPasteboard.general.string(forType: .string) else { return }
        core.scrollToBottomIfScrolled()
        send(core.encode(paste: text))
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

    private func cell(for event: NSEvent) -> GridPoint {
        let point = convert(event.locationInWindow, from: nil)
        let column = Int(((point.x - padding.left) / cellWidth).rounded(.down))
        let row = Int(((point.y - padding.top) / cellHeight).rounded(.down))
        return GridPoint(row: max(0, row), column: max(0, column))
    }

    private func modifiers(for event: NSEvent) -> VTModifiers {
        var m: VTModifiers = []
        if event.modifierFlags.contains(.shift) { m.insert(.shift) }
        if event.modifierFlags.contains(.control) { m.insert(.control) }
        if event.modifierFlags.contains(.option) { m.insert(.alt) }
        return m
    }

    override func mouseDown(with event: NSEvent) {
        claimKeyboard()
        let point = cell(for: event)

        // Shift is the universal override for "I want to select, not click" —
        // without it there is no way to copy text out of a full-screen program.
        if !event.modifierFlags.contains(.shift),
            let bytes = core.encode(
                mouse: UInt32(OVERNIGHT_VT_MOUSE_LEFT), action: UInt32(OVERNIGHT_VT_MOUSE_PRESS),
                column: point.column, row: point.row, modifiers: modifiers(for: event))
        {
            reportingButton = UInt32(OVERNIGHT_VT_MOUSE_LEFT)
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
                mouse: button, action: UInt32(OVERNIGHT_VT_MOUSE_MOVE),
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
                mouse: button, action: UInt32(OVERNIGHT_VT_MOUSE_RELEASE),
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
            ? UInt32(OVERNIGHT_VT_MOUSE_WHEEL_UP) : UInt32(OVERNIGHT_VT_MOUSE_WHEEL_DOWN)

        var bytes: [UInt8] = []
        for _ in 0..<abs(ticks) {
            guard
                let chunk = core.encode(
                    mouse: button, action: UInt32(OVERNIGHT_VT_MOUSE_PRESS),
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

    /// Convert a scroll gesture to whole lines.
    ///
    /// A trackpad reports continuous pixel deltas; a terminal scrolls in lines.
    /// Accumulating the remainder keeps a slow drag from being rounded away to
    /// nothing.
    private var scrollRemainder: CGFloat = 0

    private func wheelTicks(_ event: NSEvent) -> Int {
        if event.hasPreciseScrollingDeltas {
            scrollRemainder += event.scrollingDeltaY
            let lines = (scrollRemainder / cellHeight).rounded(.towardZero)
            scrollRemainder -= lines * cellHeight
            return Int(lines)
        }
        scrollRemainder = 0
        return Int(event.scrollingDeltaY.rounded())
    }

    // MARK: - Selection

    private func drawSelection(_ snapshot: VTSnapshot, in context: CGContext) {
        guard let selection else { return }
        let (start, end) =
            selection.anchor <= selection.head
            ? (selection.anchor, selection.head) : (selection.head, selection.anchor)

        context.setFillColor(Palette.selection.cgColor)
        for row in start.row...min(end.row, snapshot.rows - 1) {
            guard row >= 0 else { continue }
            let first = row == start.row ? start.column : 0
            let last = row == end.row ? end.column : snapshot.columns - 1
            guard last >= first else { continue }
            let point = origin(row: row, column: first)
            context.fill(
                CGRect(
                    x: point.x, y: point.y,
                    width: CGFloat(last - first + 1) * cellWidth, height: cellHeight))
        }
    }

    private func selectedText() -> String? {
        guard let selection else { return nil }
        let (start, end) =
            selection.anchor <= selection.head
            ? (selection.anchor, selection.head) : (selection.head, selection.anchor)

        return core.withSnapshot { snapshot -> String in
            var lines: [String] = []
            for row in start.row...min(end.row, snapshot.rows - 1) {
                guard row >= 0 else { continue }
                let first = max(0, row == start.row ? start.column : 0)
                let last = min(snapshot.columns - 1, row == end.row ? end.column : snapshot.columns - 1)
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

/// Colours the renderer owns. Cell colours come from the core already resolved;
/// these are the chrome around them.
enum Palette {
    static let backgroundPacked: UInt32 = 0x12_14_19
    static let background = NSColor(srgbRed: 0x12 / 255, green: 0x14 / 255, blue: 0x19 / 255, alpha: 1)
    static let cursor = NSColor(srgbRed: 0.44, green: 0.66, blue: 1.0, alpha: 0.9)
    static let selection = NSColor(srgbRed: 0.30, green: 0.42, blue: 0.62, alpha: 0.45)

    static func cgColor(_ packed: UInt32) -> CGColor {
        CGColor(
            srgbRed: CGFloat((packed >> 16) & 0xFF) / 255,
            green: CGFloat((packed >> 8) & 0xFF) / 255,
            blue: CGFloat(packed & 0xFF) / 255,
            alpha: 1)
    }
}
