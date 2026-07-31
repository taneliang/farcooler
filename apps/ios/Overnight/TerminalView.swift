import OvernightVT
import SwiftUI
import UIKit

/// The cell grid a font produces, and the insets the canvas draws inside.
///
/// Measured rather than assumed, the same reasoning as the Mac app's: this
/// device's Dynamic Type and Accessibility settings can change what "13pt
/// monospaced" measures to, and a hard-coded guess would send the host a
/// column count the screen cannot actually show.
private enum TerminalMetrics {
    static let padding = UIEdgeInsets(top: 6, left: 6, bottom: 6, right: 6)

    static func cell(_ font: UIFont) -> CGSize {
        // Every cell is the same box in a monospaced face, so one glyph's
        // measured width defines the grid.
        let width = ("M" as NSString).size(withAttributes: [.font: font]).width
        return CGSize(
            width: width.rounded(.up),
            height: (font.ascender - font.descender + font.leading).rounded(.up))
    }
}

/// Shows one terminal, live.
///
/// Everything about what an escape sequence means happened before this view
/// ever sees a byte — `TerminalSession` hands it a grid of already-resolved
/// cells. What is left here is genuinely platform work: laying that grid out
/// in points, and turning touches into the bytes a program is waiting to
/// read. It never parses an escape sequence and never decides what an arrow
/// key sends, matching the contract the C header states for every renderer.
@MainActor
struct TerminalView: View {
    @ObservedObject var connection: Connection

    @StateObject private var session: TerminalSession
    /// The terminal this screen is showing right now.
    ///
    /// A `@State` copy rather than a stored `let`, because the tab strip lets
    /// this screen point at a different terminal without ever leaving it —
    /// see `TerminalTabStrip`. Everything below that used to read `terminal`
    /// reads `current` instead, so switching tabs updates the title, the
    /// empty-state copy and the drawn grid together rather than leaving any
    /// of them talking about the terminal you tapped away from.
    @State private var current: Terminal
    @State private var ctrlArmed = false
    @State private var focusRequest = 0

    // User-configurable, unlike the Mac's fixed terminal font: see
    // Settings.swift. Read directly from the same `UserDefaults` key
    // `SettingsView`'s controls write to, so a change there is picked up the
    // next time SwiftUI redraws this screen — no delegate, no notification,
    // nothing to keep in sync by hand.
    @AppStorage(TerminalSettings.fontKey) private var fontChoice: TerminalFontChoice = .iosevka
    @AppStorage(TerminalSettings.fontSizeKey) private var fontSize: Double = TerminalSettings.defaultFontSize

    /// The cell grid the CURRENT font and size produce.
    ///
    /// Computed, not cached: a cached size is exactly what went stale the
    /// moment the settings screen changed either `@AppStorage` value out from
    /// under it, and a stale cell size is a grid that no longer lines up with
    /// what `columns(for:)`/`rows(for:)` and `draw(grid:into:size:)` assume
    /// about it.
    private var cellSize: CGSize {
        TerminalMetrics.cell(.terminal(fontChoice, size: fontSize))
    }

    init(terminal: Terminal, connection: Connection) {
        self.connection = connection
        _session = StateObject(wrappedValue: TerminalSession(terminalID: terminal.id, core: connection.core))
        _current = State(initialValue: terminal)
    }

    /// Which of several identically-labelled siblings `current` is, looked up
    /// in whichever workspace actually contains it — the same numbering
    /// `FleetView` and `TerminalTabStrip` use, so a terminal reads as
    /// "claude 2" everywhere or nowhere.
    private var currentOrdinal: Int? {
        connection.fleet.workspaces
            .first { $0.terminals.contains { $0.id == current.id } }?
            .ordinals()[current.id]
    }

    private var currentName: String { current.displayName(ordinal: currentOrdinal) }

    var body: some View {
        VStack(spacing: 0) {
            // Above the grid rather than below the key row: it is a way of
            // choosing WHICH terminal you are looking at, which belongs with
            // the title bar it effectively extends, not down with the keys
            // that type into whichever one is already open.
            TerminalTabStrip(workspaces: connection.fleet.workspaces, current: current) { tapped in
                guard tapped.id != current.id else { return }
                current = tapped
                session.switchTo(tapped.id)
            }
            GeometryReader { geo in
                phaseContent
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .task(id: GridSize(width: geo.size.width, height: geo.size.height, cell: cellSize)) {
                        await session.configure(
                            columns: columns(for: geo.size), rows: rows(for: geo.size))
                    }
            }
            Divider().overlay(Color.white.opacity(0.15))
            TerminalKeyRow(
                ctrlArmed: ctrlArmed,
                onToggleCtrl: { ctrlArmed.toggle() },
                onKey: sendKey
            )
        }
        .background(TerminalPalette.background)
        // A terminal is dark regardless of the phone's own appearance — the
        // host doesn't know or care whether this device is in Light Mode, and
        // neither should the screen showing its output.
        .preferredColorScheme(.dark)
        .navigationTitle(currentName)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                // Deliberate, and labelled as what it is. Resizing a pane
                // reflows it for everyone looking at it, so it is an action you
                // take rather than something that happens because you opened a
                // screen.
                Button {
                    Task { await session.fitPaneToViewport() }
                } label: {
                    Image(systemName: "arrow.down.right.and.arrow.up.left")
                }
                .accessibilityLabel("Fit pane to this screen")
            }
        }
        .onDisappear { session.stop() }
    }

    @ViewBuilder
    private var phaseContent: some View {
        switch session.phase {
        case .connecting:
            status(spinner: true, title: "Loading \(currentName)…")
        case .notLive:
            status(
                symbol: "moon.zzz", title: "Not live",
                message: "\(currentName) has no running pane right now.")
        case .failed(let message):
            status(symbol: "exclamationmark.triangle", title: "Could not load", message: message)
        case .live:
            if let grid = session.grid {
                live(grid: grid)
            } else {
                status(spinner: true, title: "Loading \(currentName)…")
            }
        }
    }

    private func status(
        spinner: Bool = false, symbol: String? = nil, title: String, message: String? = nil
    ) -> some View {
        VStack(spacing: 12) {
            if spinner {
                ProgressView().tint(.white)
            } else if let symbol {
                Image(systemName: symbol).font(.largeTitle).foregroundStyle(.orange)
            }
            Text(title).font(.headline).foregroundStyle(.white)
            if let message {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.white.opacity(0.7))
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 32)
            }
        }
    }

    private func live(grid: TerminalGrid) -> some View {
        ZStack(alignment: .topLeading) {
            Canvas { context, size in draw(grid: grid, into: context, size: size) }
            // A UIKit view whose only job is to be the thing the software
            // keyboard is attached to. It carries no visible state of its
            // own — every character it reports goes straight to the host and
            // back through the poll, which is the only place this view ever
            // draws what was typed.
            // Filling the terminal rather than hiding in a corner.
            //
            // It was a 1×1 view at 1% opacity with the tap handled by the SwiftUI
            // stack around it, and it never reliably took first responder: a view
            // that small and that transparent is not something UIKit is eager to
            // give the keyboard to, and the tap had to travel through a Canvas to
            // reach it. Sizing it to the area it represents makes it an ordinary
            // view that can be tapped and focused, and it is still invisible
            // because it draws nothing.
            KeystrokeField(focusRequest: focusRequest, onInsertText: insert, onDeleteBackward: {
                sendKey(UInt32(OVERNIGHT_VT_KEY_BACKSPACE))
            })
        }
        .onAppear { focusRequest += 1 }
    }

    // MARK: - Drawing

    private func draw(grid: TerminalGrid, into context: GraphicsContext, size: CGSize) {
        context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(TerminalPalette.background))
        let padding = TerminalMetrics.padding
        let cell = cellSize

        // Scaled by hand — every position and font size below is multiplied
        // by `scale` directly — rather than through `GraphicsContext.scaleBy`.
        //
        // The cell size comes from a fixed font, which is right for a Mac window
        // sized to its own terminal and wrong here: the phone renders whatever
        // grid the HOST has — 75 columns is normal — and at 13pt that is far
        // wider than any phone. `scaleBy` looks like the right tool and reads
        // as one, but it does not touch glyphs drawn through `context.draw(_
        // text: Text, at:anchor:)`: only `size` and `content` were logged
        // against a live 40-column pane, and a scale of 0.69 was computed
        // correctly and applied to the context on every frame while the
        // glyphs kept landing at their full, untransformed size — the CTM
        // reached the `Path`-based fills below but never the text. So the
        // fills and the text now agree by construction: both are laid out in
        // coordinates that already have `scale` baked in, which is correct
        // regardless of which drawing calls a given `GraphicsContext` happens
        // to honour its own transform for.
        //
        // Scaling down rather than reflowing, because reflowing means resizing
        // the pane, and the pane is shared: making it fit here made everyone
        // else's layout collapse. Small but complete beats large and cropped —
        // and `Fit pane to phone` is there for when you do want it reflowed.
        let content = CGSize(
            width: padding.left + padding.right + CGFloat(grid.columns) * cell.width,
            height: padding.top + padding.bottom + CGFloat(grid.rows) * cell.height)
        guard content.width > 0, content.height > 0 else { return }
        let scale = min(size.width / content.width, size.height / content.height, 1)

        let scaledCell = CGSize(width: cell.width * scale, height: cell.height * scale)
        let originX = padding.left * scale
        let originY = padding.top * scale
        // The chosen typeface, not a hard-coded system one — see
        // Settings.swift. Scaled the same way the cell itself is: a font
        // built at the unscaled `fontSize` would draw glyphs too big for the
        // very cell `scale` just shrank to make everything fit.
        let regularFont = Font.terminal(fontChoice, size: fontSize * scale)
        let boldFont = Font.terminal(fontChoice, size: fontSize * scale, bold: true)

        for row in 0..<grid.rows {
            let y = originY + CGFloat(row) * scaledCell.height

            // Background first, batched into runs: a wide stretch of the
            // terminal's own default background is the common case, and
            // skipping it entirely is one comparison instead of one fill per
            // cell across a mostly-empty screen.
            var column = 0
            while column < grid.columns {
                let background = grid[row, column].background
                var end = column + 1
                while end < grid.columns, grid[row, end].background == background { end += 1 }
                if background != TerminalPalette.background {
                    let rect = CGRect(
                        x: originX + CGFloat(column) * scaledCell.width, y: y,
                        width: CGFloat(end - column) * scaledCell.width, height: scaledCell.height)
                    context.fill(Path(rect), with: .color(background))
                }
                column = end
            }

            column = 0
            while column < grid.columns {
                let occupant = grid[row, column]
                if let character = occupant.character {
                    let point = CGPoint(x: originX + CGFloat(column) * scaledCell.width, y: y)
                    context.draw(
                        Text(String(character))
                            .font(occupant.bold ? boldFont : regularFont)
                            .foregroundColor(occupant.foreground),
                        at: point, anchor: .topLeading)
                }
                // A wide character's neighbour is its own spacer; drawing it
                // too would put a second, blank glyph where the wide one
                // already reaches.
                column += occupant.wide ? 2 : 1
            }
        }

        guard grid.cursorRow < grid.rows, grid.cursorColumn < grid.columns else { return }
        let cursorCell = grid[grid.cursorRow, grid.cursorColumn]
        let cursorRect = CGRect(
            x: originX + CGFloat(grid.cursorColumn) * scaledCell.width,
            y: originY + CGFloat(grid.cursorRow) * scaledCell.height,
            width: cursorCell.wide ? scaledCell.width * 2 : scaledCell.width,
            height: scaledCell.height)
        context.fill(Path(cursorRect), with: .color(TerminalPalette.cursor))
        if let character = cursorCell.character {
            context.draw(
                Text(String(character)).font(regularFont).foregroundColor(TerminalPalette.background),
                at: cursorRect.origin, anchor: .topLeading)
        }
    }

    // MARK: - Geometry

    private func columns(for size: CGSize) -> Int {
        let padding = TerminalMetrics.padding
        let usable = size.width - padding.left - padding.right
        return max(1, Int((usable / cellSize.width).rounded(.down)))
    }

    private func rows(for size: CGSize) -> Int {
        let padding = TerminalMetrics.padding
        let usable = size.height - padding.top - padding.bottom
        return max(1, Int((usable / cellSize.height).rounded(.down)))
    }

    // MARK: - Input

    /// Route one burst of typed text, special-casing the newline the
    /// software keyboard's Return key produces.
    ///
    /// `insertText("\n")` is what UIKit sends for a keyboard Return tap, but
    /// passing the scalar straight through would send a bare 0x0A — a literal
    /// newline character rather than the Enter *keystroke* a program expects,
    /// and the two are not always interchangeable under a raw pty. Routing it
    /// through the same encoder the Return button uses keeps the keyboard's
    /// own Return key and the accessory row's agreeing with each other.
    private func insert(_ text: String) {
        if text == "\n" {
            sendKey(UInt32(OVERNIGHT_VT_KEY_ENTER))
            return
        }
        let modifiers = consumeCtrl()
        Task { await session.send(text: text, modifiers: modifiers) }
    }

    private func sendKey(_ key: UInt32) {
        let modifiers = consumeCtrl()
        Task { await session.send(key: key, modifiers: modifiers) }
    }

    /// Ctrl is a toggle, not a held key — there is nothing on a touchscreen
    /// that behaves like holding a modifier down. So it applies to exactly
    /// the next key and then clears itself, the same shape as Shift-lock on
    /// a physical keyboard with only one hand.
    private func consumeCtrl() -> VTModifiers {
        defer { ctrlArmed = false }
        return ctrlArmed ? .control : []
    }

    /// What `columns(for:)`/`rows(for:)` depend on — the view's own size, AND
    /// the cell a font produces. `cell` is here so a font or size change in
    /// Settings re-runs `session.configure`, not just the drawing: a viewport
    /// that fit 80 columns at 13pt fits fewer at 18pt, and the pane this
    /// screen would ask to be sized to on a future `fitPaneToViewport` should
    /// reflect the font actually on screen, not whatever it was measured at
    /// when this screen first appeared.
    private struct GridSize: Equatable {
        var width: Double
        var height: Double
        var cell: CGSize
    }
}

/// The row of keys a terminal needs and a phone's keyboard does not have.
private struct TerminalKeyRow: View {
    let ctrlArmed: Bool
    let onToggleCtrl: () -> Void
    let onKey: (UInt32) -> Void

    var body: some View {
        HStack(spacing: 6) {
            key("esc") { onKey(UInt32(OVERNIGHT_VT_KEY_ESCAPE)) }
            key("tab") { onKey(UInt32(OVERNIGHT_VT_KEY_TAB)) }
            key("ctrl", armed: ctrlArmed, action: onToggleCtrl)
            Spacer(minLength: 2)
            key("←") { onKey(UInt32(OVERNIGHT_VT_KEY_LEFT)) }
            key("↓") { onKey(UInt32(OVERNIGHT_VT_KEY_DOWN)) }
            key("↑") { onKey(UInt32(OVERNIGHT_VT_KEY_UP)) }
            key("→") { onKey(UInt32(OVERNIGHT_VT_KEY_RIGHT)) }
            Spacer(minLength: 2)
            key("⏎") { onKey(UInt32(OVERNIGHT_VT_KEY_ENTER)) }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(Color(white: 0.1))
    }

    private func key(_ label: String, armed: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(label)
                .font(.system(.callout, design: .monospaced))
                .frame(minWidth: 30, minHeight: 30)
                .foregroundStyle(armed ? Color.black : Color.white)
                .background(armed ? Color.white : Color.white.opacity(0.12))
                .clipShape(RoundedRectangle(cornerRadius: 6))
        }
        .buttonStyle(.plain)
    }
}

/// Attaches the software keyboard to a view with no text of its own.
///
/// `UIKeyInput` rather than the full `UITextInput` a `UITextField` needs: it
/// is the smaller protocol that still gets a keyboard attached, and this view
/// has no text to hold, no cursor to place, and no selection to track — the
/// terminal grid is the only place any of those are ever drawn.
private struct KeystrokeField: UIViewRepresentable {
    var focusRequest: Int
    var onInsertText: (String) -> Void
    var onDeleteBackward: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> KeystrokeSink {
        let view = KeystrokeSink()
        view.onInsertText = onInsertText
        view.onDeleteBackward = onDeleteBackward
        view.backgroundColor = .clear
        // Handled here rather than by a SwiftUI gesture above it: the view that
        // wants the keyboard is the one that should hear the tap asking for it.
        view.addGestureRecognizer(
            UITapGestureRecognizer(target: view, action: #selector(KeystrokeSink.focus)))
        return view
    }

    func updateUIView(_ uiView: KeystrokeSink, context: Context) {
        uiView.onInsertText = onInsertText
        uiView.onDeleteBackward = onDeleteBackward

        // `updateUIView` runs on every poll, not just on a tap — the grid it
        // sits beside changes every second. Only actually re-focus when
        // `focusRequest` itself moved, or an on-screen keyboard the user
        // deliberately dismissed would be pulled back up on the next tick.
        guard context.coordinator.lastFocusRequest != focusRequest else { return }
        context.coordinator.lastFocusRequest = focusRequest
        DispatchQueue.main.async { uiView.becomeFirstResponder() }
    }

    final class Coordinator {
        var lastFocusRequest = -1
    }
}

private final class KeystrokeSink: UIView, UIKeyInput {
    var onInsertText: ((String) -> Void)?
    var onDeleteBackward: (() -> Void)?

    override var canBecomeFirstResponder: Bool { true }
    var hasText: Bool { true }

    @objc func focus() { becomeFirstResponder() }

    func insertText(_ text: String) { onInsertText?(text) }
    func deleteBackward() { onDeleteBackward?() }

    // Every trait below exists to stop iOS from "helping": autocorrect
    // rewriting a flag, smart quotes producing a character no shell
    // recognises, autocapitalization upper-casing the first letter of a
    // command nobody typed that way.
    var autocorrectionType: UITextAutocorrectionType = .no
    var autocapitalizationType: UITextAutocapitalizationType = .none
    var smartQuotesType: UITextSmartQuotesType = .no
    var smartDashesType: UITextSmartDashesType = .no
    var smartInsertDeleteType: UITextSmartInsertDeleteType = .no
    var spellCheckingType: UITextSpellCheckingType = .no
    var keyboardType: UIKeyboardType = .asciiCapable
    var returnKeyType: UIReturnKeyType = .send
}
