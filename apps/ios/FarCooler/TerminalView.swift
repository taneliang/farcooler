import FarCoolerVT
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
    @Environment(\.scenePhase) private var scenePhase

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
    @State private var altArmed = false
    @State private var focusRequest = 0
    @State private var dismissRequest = 0
    @State private var showWorkspaceList = false

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

    /// The machines to switch between, offered inside the switcher sheet.
    ///
    /// Optional because this screen works perfectly well without one — it is
    /// the terminal, not the fleet — and because the previews and the demo
    /// path construct it with no store at all.
    let hosts: HostStore?

    init(terminal: Terminal, connection: Connection, hosts: HostStore? = nil) {
        self.connection = connection
        self.hosts = hosts
        _session = StateObject(wrappedValue: TerminalSession(terminalID: terminal.id, core: connection.core))
        _current = State(initialValue: terminal)
    }

    /// The workspace `current` actually lives in, looked up fresh each time
    /// rather than carried in from wherever `current` was set — a workspace
    /// the tab strip or the switcher sheet points at is only ever known to
    /// this screen by its terminal's id.
    /// The terminal as the daemon describes it RIGHT NOW.
    ///
    /// `current` is a `@State` copy, taken when this screen opened and updated
    /// by the tab strip. That is right for identity and wrong for anything that
    /// changes underneath it — pane mode above all. Switching to chat left the
    /// copy still saying "terminal", so the button asked for the same switch
    /// every time, the screen kept drawing a VT grid, and the change only
    /// appeared after navigating away and back, which rebuilt the copy.
    private var live: Terminal {
        guard let workspace = currentWorkspace?.id else { return current }
        return connection.terminal(current.id, in: workspace) ?? current
    }

    private var currentWorkspace: Workspace? {
        connection.fleet.workspaces.first { $0.terminals.contains { $0.id == current.id } }
    }

    /// Which of several identically-labeled siblings `current` is — the
    /// same numbering `FleetView` and `TerminalTabStrip` use, so a terminal
    /// reads as "claude 2" everywhere or nowhere.
    private var currentOrdinal: Int? {
        currentWorkspace?.ordinals()[current.id]
    }

    private var currentName: String { current.displayName(ordinal: currentOrdinal) }

    var body: some View {
        VStack(spacing: 0) {
            // Chosen by `terminal.isAgentPane`, which the daemon sets — never
            // derived here, the same rule that keeps `activity` and
            // `agentMode` as reported rather than guessed at. `AgentView` is
            // given `current.id` as its SwiftUI identity: a tab switch
            // between two agent panes recreates it rather than retargeting
            // an existing `AgentStream`, which is simpler than the terminal
            // side's `TerminalSession.switchTo` and correct here because an
            // agent session has no live ssh channel to hand off — a fresh
            // subscribe from seq 0 costs one round trip, not a stream.
            if live.isAgentPane {
                AgentView(terminalID: current.id, workspaceID: currentWorkspace?.id, connection: connection)
                    .id(current.id)
            } else {
                GeometryReader { geo in
                    phaseContent(size: geo.size)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .task(id: GridSize(width: geo.size.width, height: geo.size.height, cell: cellSize)) {
                            await session.configure(
                                columns: columns(for: geo.size), rows: rows(for: geo.size))
                        }
                }
            }
            // Both bars live at the bottom, in thumb reach. The tab strip used
            // to sit under the title, which put the one control you use
            // constantly — switching terminal — at the far end of the screen
            // from the hand holding the phone. That still holds; what changed
            // is that it floats now instead of sitting on a slab of its own.
        }
        // The strip lives in the bottom safe area, not in the stack and not in
        // the keyboard's accessory.
        //
        // In the stack it sat under the keyboard when one came up. In the
        // accessory it rode up correctly but an accessory floats OVER content,
        // so the terminal was laid out as though the strip were not there and
        // the grid's last line ended up behind it. A safe-area inset is the one
        // placement the layout actually accounts for: the grid shrinks by
        // exactly the strip's height, and keyboard avoidance carries both up
        // together.
        .safeAreaInset(edge: .bottom, spacing: 0) {
            // Spaced on BOTH sides.
            //
            // The inset sits inside the terminal's own background, so with no
            // padding the strip read as welded to the bottom of the grid, and
            // with none below it crowded the key row that rides up on the
            // keyboard. Two floating bars need to look like two floating bars:
            // air above, air below, and the grid ending cleanly before either.
            TerminalTabStrip(
                workspaces: connection.fleet.workspaces, current: current, onSelect: select
            )
            .padding(.top, 8)
            .padding(.bottom, 10)
        }
        .background(TerminalPalette.background)
        // A terminal is dark regardless of the phone's own appearance — the
        // host doesn't know or care whether this device is in Light Mode, and
        // neither should the screen showing its output.
        .preferredColorScheme(.dark)
        // The task and its branch, not `currentName` — the terminal already
        // names itself in its tab strip chip, and repeating it here would
        // waste the one line of title bar a phone has on something already on
        // screen.
        // Same reasoning as the Mac's `Workspace.windowTitle`/`windowSubtitle`
        // (apps/macos/Sources/FarCooler/Model.swift): the place you are is
        // the workspace, and that's true regardless of which of its
        // terminals is focused. The host itself is left out — unlike the
        // Mac, this screen is already scoped to one host by the time you're
        // looking at it, so naming it again would answer a question nobody
        // is asking here.
        .navigationTitle(currentWorkspace?.task ?? currentName)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            // Only when there is something wrong, and then unmissably.
            //
            // The machine bar carries this permanently, but the bar lives on
            // the worktree list and this screen is the one people actually sit
            // on — a terminal that has quietly stopped updating is exactly
            // where a frozen link is noticed and, before this, the only place
            // it could not be acted on without first opening the switcher.
            if connection.phase != .connected {
                ToolbarItem(placement: .topBarLeading) {
                    LinkStatusChip(connection: connection)
                }
            }

            ToolbarItem(placement: .principal) {
                VStack(spacing: 0) {
                    Text(currentWorkspace?.task ?? currentName)
                        .font(.headline)
                        .lineLimit(1)
                    if let branch = currentWorkspace?.branch {
                        Text(branch)
                            .font(.caption2.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
            }
            // Terminal or chat, on the pane that can be either.
            //
            // The Mac puts this in the pane header; a phone has no pane header,
            // so it goes in the bar that is already this screen's chrome. Shown
            // only where it would work — `canSwitchPaneMode` already reflects
            // the daemon's own registry-backed `chat_capable` check, so a pane
            // whose agent has no adapter never gets the button in the first
            // place; the shim itself is told which adapter to speak via
            // `--preset`, rather than assuming one for everything.
            if live.canSwitchPaneMode {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        Task {
                            await connection.setPaneMode(
                                live, to: live.isAgentPane ? "terminal" : "agent")
                        }
                    } label: {
                        Image(
                            systemName: live.isAgentPane
                                ? "terminal" : "bubble.left.and.text.bubble.right")
                    }
                    .accessibilityLabel(live.isAgentPane ? "Show the terminal" : "Show the chat")
                }
            }

            ToolbarItem(placement: .topBarTrailing) {
                Button { showWorkspaceList = true } label: {
                    Image(systemName: "square.stack")
                }
                .accessibilityLabel("Switch terminal")
            }
        }
        .sheet(isPresented: $showWorkspaceList) {
            NavigationStack {
                WorkspaceListView(
                    connection: connection,
                    onSelect: { terminal in
                        select(terminal)
                        showWorkspaceList = false
                    },
                    onDismiss: { showWorkspaceList = false },
                    hosts: hosts
                )
                .navigationTitle("Worktrees")
                .navigationBarTitleDisplayMode(.inline)
            }
        }
        // Which pane is on screen, so a banner about THIS one is suppressed
        // while banners about the others still arrive — see `Notifier`.
        //
        // It is also what ends `done`: a pane on screen has been read, so each
        // of these three moments marks it seen rather than waiting up to three
        // seconds for the poll to notice. See `Connection.markVisibleSeen`.
        .onAppear {
            Notifier.shared.visibleTerminal = current.id
            Task { await connection.markVisibleSeen() }
        }
        .onChange(of: current.id) { _, id in
            Notifier.shared.visibleTerminal = id
            Task { await connection.markVisibleSeen() }
        }
        .onDisappear {
            session.stop()
            Notifier.shared.visibleTerminal = nil
        }
        // Returning to the foreground carries no geometry of its own — this
        // screen's size has not changed — but the pane is shared, and someone
        // on the Mac could have resized the shared window while this device
        // was backgrounded and not watching. `reassertSize` re-asks for the
        // size already on file rather than computing a new one.
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active else { return }
            session.reassertSize()
            // Coming back to the app is reading whatever it comes back to. The
            // notification did its job while the phone was away; leaving the
            // pane flagged afterwards makes you dismiss the same news twice.
            Task { await connection.markVisibleSeen() }
        }
        // The link under this pane was replaced.
        //
        // A stream is a second SSH channel on the session that just died, so
        // everything this screen had open went with it. `TerminalSession`
        // survives that on its own by falling back to polling, which is the
        // right behavior and the slower path; this puts it back on the stream
        // now that there is one to be on.
        .onChange(of: connection.reconnectGeneration) { _, _ in
            session.relink()
        }
    }

    @ViewBuilder
    private func phaseContent(size: CGSize) -> some View {
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
                live(grid: grid, size: size)
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

    private func live(grid: TerminalGrid, size: CGSize) -> some View {
        ZStack(alignment: .topLeading) {
            Canvas { context, canvasSize in draw(grid: grid, into: context, size: canvasSize) }
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
            //
            // It also carries the scroll gesture, for the same reason: it is
            // the view that already owns every touch landing on the terminal.
            // A `DragGesture` layered on top in SwiftUI would be racing this
            // view's own `UITapGestureRecognizer` for the same touches rather
            // than cooperating with it, so the pan recognizer lives on the
            // same UIView and the tap is told to lose that race explicitly —
            // see `KeystrokeSink.setup()`.
            KeystrokeField(
                focusRequest: focusRequest,
                dismissRequest: dismissRequest,
                cellHeight: gridLayout(for: grid, in: size).cell.height,
                onInsertText: insert,
                onDeleteBackward: { sendKey(UInt32(FARCOOLER_VT_KEY_BACKSPACE)) },
                onScroll: { lines, point in
                    let target = cell(at: point, grid: grid, size: size)
                    Task { await session.scroll(lines: lines, column: target.column, row: target.row) }
                },
                accessory: AnyView(
                    TerminalKeyRow(
                        ctrlArmed: ctrlArmed,
                        altArmed: altArmed,
                        onToggleCtrl: { ctrlArmed.toggle() },
                        onToggleAlt: { altArmed.toggle() },
                        onKey: sendKey,
                        onDismiss: { dismissRequest += 1 }
                    )
                )
            )
        }
        // Bounded to the terminal, and clipped to it.
        //
        // `KeystrokeSink` is an invisible `UIView` with no intrinsic size, so
        // nothing stops it being handed more room than the grid it belongs to —
        // and it owns every touch that lands on it. That was harmless while the
        // tab strip sat above the terminal, and became a bug the moment the
        // strip moved below: the sink covered the chips, so tapping a terminal
        // to switch to it did nothing at all.
        .frame(width: size.width, height: size.height)
        .clipped()
        .onAppear { focusRequest += 1 }
    }

    // MARK: - Drawing

    /// Where the grid sits inside the canvas, and at what scale — the same
    /// numbers `draw(grid:into:size:)` lays glyphs out with, and what
    /// `cell(at:grid:size:)` inverts to turn a touch back into a column and
    /// row. Kept as one calculation because the two had a habit of drifting
    /// apart the moment they were two.
    private struct GridLayout {
        var scale: CGFloat
        var cell: CGSize
        var origin: CGPoint
    }

    /// Almost never a straight reflow: the pane resizes itself to roughly fit
    /// (see `TerminalSession.configure`), but the two round trips that takes —
    /// asking the host, the host reflowing tmux — leave a gap where the grid
    /// on screen is still the OLD size, and this scales that leftover
    /// mismatch down rather than cropping it. `scale` is 1 the rest of the
    /// time.
    private func gridLayout(for grid: TerminalGrid, in size: CGSize) -> GridLayout {
        let padding = TerminalMetrics.padding
        let cell = cellSize
        let content = CGSize(
            width: padding.left + padding.right + CGFloat(grid.columns) * cell.width,
            height: padding.top + padding.bottom + CGFloat(grid.rows) * cell.height)
        guard content.width > 0, content.height > 0 else {
            return GridLayout(scale: 1, cell: cell, origin: .zero)
        }
        let scale = min(size.width / content.width, size.height / content.height, 1)
        return GridLayout(
            scale: scale,
            cell: CGSize(width: cell.width * scale, height: cell.height * scale),
            origin: CGPoint(x: padding.left * scale, y: padding.top * scale))
    }

    /// The grid cell under a point in the canvas's own coordinates, clamped
    /// onto the grid so a touch a pixel past the last row still lands
    /// somewhere real rather than off the edge of `grid`'s backing array.
    private func cell(at point: CGPoint, grid: TerminalGrid, size: CGSize) -> (column: Int, row: Int) {
        let layout = gridLayout(for: grid, in: size)
        guard layout.cell.width > 0, layout.cell.height > 0 else { return (0, 0) }
        let column = Int(((point.x - layout.origin.x) / layout.cell.width).rounded(.down))
        let row = Int(((point.y - layout.origin.y) / layout.cell.height).rounded(.down))
        return (
            min(max(column, 0), max(grid.columns - 1, 0)),
            min(max(row, 0), max(grid.rows - 1, 0))
        )
    }

    private func draw(grid: TerminalGrid, into context: GraphicsContext, size: CGSize) {
        context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(TerminalPalette.background))
        let layout = gridLayout(for: grid, in: size)

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
        guard layout.cell.width > 0, layout.cell.height > 0 else { return }
        let scale = layout.scale
        let scaledCell = layout.cell
        let originX = layout.origin.x
        let originY = layout.origin.y
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

    /// Point this screen at a different terminal without leaving it — what
    /// both the tab strip and the switcher sheet's rows do on a tap, kept in
    /// one place so the two never disagree about what "switching" means.
    private func select(_ terminal: Terminal) {
        guard terminal.id != current.id else { return }
        current = terminal
        // An agent pane draws nothing from `TerminalSession` — there is no
        // ssh stream to retarget, and leaving one open on the terminal just
        // tapped away from would keep a channel alive for a pane nothing is
        // showing. `stop()` releases it the same way leaving this screen
        // entirely does in `.onDisappear`; the branch below still runs the
        // ordinary retarget whenever the newly selected tab is a terminal.
        if terminal.isAgentPane {
            session.stop()
        } else {
            session.switchTo(terminal.id)
        }
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
            sendKey(UInt32(FARCOOLER_VT_KEY_ENTER))
            return
        }
        let modifiers = consumeModifiers()
        Task { await session.send(text: text, modifiers: modifiers) }
    }

    private func sendKey(_ key: UInt32) {
        let modifiers = consumeModifiers()
        Task { await session.send(key: key, modifiers: modifiers) }
    }

    /// Ctrl is a toggle, not a held key — there is nothing on a touchscreen
    /// that behaves like holding a modifier down. So it applies to exactly
    /// the next key and then clears itself, the same shape as Shift-lock on
    /// a physical keyboard with only one hand.
    /// Ctrl and Alt are toggles, not held keys — there is nothing on a
    /// touchscreen that behaves like holding a modifier down. So each applies
    /// to exactly the next key and then clears itself, the same shape as
    /// Shift-lock on a physical keyboard with only one hand.
    private func consumeModifiers() -> VTModifiers {
        defer {
            ctrlArmed = false
            altArmed = false
        }
        var mods: VTModifiers = []
        if ctrlArmed { mods.insert(.control) }
        if altArmed { mods.insert(.alt) }
        return mods
    }

    /// What `columns(for:)`/`rows(for:)` depend on — the view's own size, AND
    /// the cell a font produces. `cell` is here so a font or size change in
    /// Settings re-runs `session.configure`, not just the drawing: a viewport
    /// that fit 80 columns at 13pt fits fewer at 18pt, and the pane this
    /// screen asks the host to resize to should reflect the font actually on
    /// screen, not whatever it was measured at when this screen first
    /// appeared.
    private struct GridSize: Equatable {
        var width: Double
        var height: Double
        var cell: CGSize
    }
}

/// The row of keys a terminal needs and a phone's keyboard does not have.
///
/// Styled as a keyboard accessory, not a strip of custom chrome: system
/// materials and button styles rather than a hand-picked grey and hand-rolled
/// pressed states, so it reads as part of iOS rather than as a widget
/// floating on top of it. `.bordered`/`.borderedProminent` are what give
/// every key its pressed-state animation for free — there is no
/// `isPressed`-driven fill anywhere here.
private struct TerminalKeyRow: View {
    let ctrlArmed: Bool
    let altArmed: Bool
    let onToggleCtrl: () -> Void
    let onToggleAlt: () -> Void
    let onKey: (UInt32) -> Void
    let onDismiss: () -> Void

    /// Keys share the width rather than each claiming their own.
    ///
    /// Nine keys sized from their own content overflowed a phone once — a
    /// bordered button pads whatever you hand it, so a 34pt legend became a
    /// 58pt key. The row did not clip on its own: it widened the stack it was
    /// in, so the terminal ABOVE it lost characters off both edges.
    /// `maxWidth: .infinity` makes overflow impossible to express.
    private static let keyHeight: CGFloat = 40

    var body: some View {
        HStack(spacing: 5) {
            key { onKey(UInt32(FARCOOLER_VT_KEY_ESCAPE)) } label: { glyph("escape") }
            key { onKey(UInt32(FARCOOLER_VT_KEY_TAB)) } label: { glyph("arrow.right.to.line") }
            key(filled: ctrlArmed, action: onToggleCtrl) { glyph("control") }
            key(filled: altArmed, action: onToggleAlt) { glyph("option") }
            // Held, each arrow becomes the jump it is the small version of.
            // A phone has no room for eight more keys and no modifier to hide
            // them behind, and holding a direction to go further in it is the
            // gesture people already have for exactly this.
            // Solid arrows rather than chevrons, because `control` IS a
            // chevron: ⌃ beside a chevron-up meant two keys with the same
            // glyph sitting four apart in the same row.
            arrow("arrow.left", tap: FARCOOLER_VT_KEY_LEFT, hold: FARCOOLER_VT_KEY_HOME)
            arrow("arrow.down", tap: FARCOOLER_VT_KEY_DOWN, hold: FARCOOLER_VT_KEY_PAGE_DOWN)
            arrow("arrow.up", tap: FARCOOLER_VT_KEY_UP, hold: FARCOOLER_VT_KEY_PAGE_UP)
            arrow("arrow.right", tap: FARCOOLER_VT_KEY_RIGHT, hold: FARCOOLER_VT_KEY_END)
            // Putting the keyboard away, which this row is otherwise the only
            // thing standing in the way of: it lives above the keyboard, so it
            // goes when the keyboard does, and without a way to dismiss from
            // here there is nowhere else to ask from.
            key(action: onDismiss) { glyph("keyboard.chevron.compact.down") }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 7)
        // Glass, like the composer next door.
        //
        // This used to take the `UIInputView`'s own backdrop, so that it read
        // as the keyboard's top edge rather than a slab resting on it. That was
        // right when the keyboard's edge was a flat bar; on iOS 26 the thing
        // resting above a keyboard is a floating glass surface, and a squared
        // strip against a rounded composer looked like two releases of the app
        // at once.
        //
        // No Return key. The software keyboard already has one, and the row's
        // whole job is the keys a phone keyboard does not have.
        .modifier(GlassSurface(radius: 18))
        .padding(.horizontal, 8)
        // Clear of the keyboard, not resting on it.
        //
        // A floating bar whose bottom edge is flush against the keyboard's top
        // edge is not floating — it reads as a strip welded to the keyboard
        // with rounded corners drawn on, which is worse than either honest
        // option. The gap is what says the two are different surfaces.
        .padding(.bottom, 10)
        .padding(.top, 2)
    }

    /// An arrow that means one thing tapped and a bigger version of the same
    /// thing held.
    private func arrow(_ symbol: String, tap: UInt32, hold: UInt32) -> some View {
        key(action: { onKey(tap) }, onHold: { onKey(hold) }) { glyph(symbol) }
    }

    /// One key: a rounded rectangle, because that is what a key looks like
    /// here. The system's bordered style rounds to a capsule at these
    /// proportions, which reads as a row of pills rather than a keyboard.
    private func key<Label: View>(
        filled: Bool = false,
        action: @escaping () -> Void,
        onHold: (() -> Void)? = nil,
        @ViewBuilder label: () -> Label
    ) -> some View {
        Button(action: action) {
            label()
                .frame(maxWidth: .infinity, minHeight: Self.keyHeight)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(filled ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(Color(uiColor: .systemGray3)))
                )
                .foregroundStyle(filled ? Color.white : Color.primary)
        }
        .buttonStyle(.plain)
        // A long press that fires once, rather than a repeat: these send a
        // jump, and a jump that repeats while your thumb rests on it would
        // scroll somewhere nobody asked to be.
        .onLongPressGesture(minimumDuration: 0.35) { onHold?() }
    }

    private func glyph(_ symbol: String) -> some View {
        Image(systemName: symbol).font(.system(size: 15, weight: .medium))
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
    /// Bumped to put the keyboard away. A counter rather than a boolean for
    /// the same reason `focusRequest` is one: what matters is that a fresh
    /// request happened, not what state anything is in.
    var dismissRequest: Int
    /// The height, in points, one terminal row is actually drawn at right
    /// now — what a drag on this view is converted to lines against. Passed
    /// in rather than measured here, because only `TerminalView` knows the
    /// current font, size and scale that produced it.
    var cellHeight: CGFloat
    var onInsertText: (String) -> Void
    var onDeleteBackward: () -> Void
    var onScroll: (Int, CGPoint) -> Void
    /// The key row, handed to the system as the keyboard's accessory so it is
    /// drawn as part of the keyboard rather than as a strip above one.
    var accessory: AnyView

    func makeCoordinator() -> Coordinator { Coordinator() }

    /// The accessory's exact height. Stated rather than self-sized, because a
    /// `UIInputView` that sizes itself ends up taller than its content and the
    /// difference is transparent — which is what put a strip of window between
    /// the row and the keyboard and made the two look unrelated.
    private static let accessoryHeight: CGFloat = 52

    func makeUIView(context: Context) -> KeystrokeSink {
        let view = KeystrokeSink()
        view.onInsertText = onInsertText
        view.onDeleteBackward = onDeleteBackward
        view.onScroll = onScroll
        view.cellHeight = cellHeight
        view.backgroundColor = .clear

        let host = UIHostingController(rootView: accessory)
        host.view.backgroundColor = .clear
        // The accessory sits ON the keyboard, so the home indicator is the
        // keyboard's problem and not this row's. Left alone, the hosting
        // controller adds the bottom inset itself and the row floats.
        host.safeAreaRegions = []
        host.view.translatesAutoresizingMaskIntoConstraints = false
        context.coordinator.accessoryHost = host

        // `.keyboard` style draws the system keyboard's own background, which
        // is what makes the row belong to the keyboard rather than resemble it.
        let bar = UIInputView(
            frame: CGRect(x: 0, y: 0, width: 0, height: Self.accessoryHeight),
            inputViewStyle: .keyboard)
        bar.autoresizingMask = .flexibleWidth
        bar.addSubview(host.view)
        NSLayoutConstraint.activate([
            host.view.leadingAnchor.constraint(equalTo: bar.leadingAnchor),
            host.view.trailingAnchor.constraint(equalTo: bar.trailingAnchor),
            host.view.topAnchor.constraint(equalTo: bar.topAnchor),
            host.view.bottomAnchor.constraint(equalTo: bar.bottomAnchor),
        ])
        view.accessory = bar
        return view
    }

    func updateUIView(_ uiView: KeystrokeSink, context: Context) {
        uiView.onInsertText = onInsertText
        uiView.onDeleteBackward = onDeleteBackward
        uiView.onScroll = onScroll
        uiView.cellHeight = cellHeight
        context.coordinator.accessoryHost?.rootView = accessory

        // `updateUIView` runs on every poll, not just on a tap — the grid it
        // sits beside changes every second. Only actually re-focus when
        // `focusRequest` itself moved, or an on-screen keyboard the user
        // deliberately dismissed would be pulled back up on the next tick.
        if context.coordinator.lastDismissRequest != dismissRequest {
            context.coordinator.lastDismissRequest = dismissRequest
            DispatchQueue.main.async { uiView.resignFirstResponder() }
            return
        }

        guard context.coordinator.lastFocusRequest != focusRequest else { return }
        context.coordinator.lastFocusRequest = focusRequest
        DispatchQueue.main.async { uiView.becomeFirstResponder() }
    }

    final class Coordinator {
        var lastFocusRequest = -1
        var lastDismissRequest = 0
        /// Retained because nothing else owns it: the input view holds the
        /// hosting controller's VIEW, and a controller referenced only through
        /// its own view is deallocated along with its SwiftUI state.
        var accessoryHost: UIHostingController<AnyView>?
    }
}

private final class KeystrokeSink: UIView, UIKeyInput {
    var onInsertText: ((String) -> Void)?
    var onDeleteBackward: (() -> Void)?
    var onScroll: ((Int, CGPoint) -> Void)?
    var cellHeight: CGFloat = 16
    /// The key row, shown by the system as part of the keyboard.
    var accessory: UIView?

    override var inputAccessoryView: UIView? { accessory }


    override init(frame: CGRect) {
        super.init(frame: frame)
        setup()
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    /// Both gesture recognizers this view carries, wired together here so the
    /// relationship between them is set up in exactly one place.
    ///
    /// The tap is what asks for the keyboard; the pan is what scrolls. Both
    /// live on this view — the one thing on screen that already owns every
    /// touch landing on the terminal — rather than the pan being a SwiftUI
    /// `DragGesture` layered above it, which would be racing this view's own
    /// recognizer for the same touches instead of cooperating with it.
    /// `require(toFail:)` is what keeps a drag that scrolls from also being
    /// read as the tap that raises the keyboard: the tap does not fire until
    /// the pan has definitively not started.
    private func setup() {
        let tap = UITapGestureRecognizer(target: self, action: #selector(focus))
        let pan = UIPanGestureRecognizer(target: self, action: #selector(handlePan))
        tap.require(toFail: pan)
        addGestureRecognizer(tap)
        addGestureRecognizer(pan)
    }

    override var canBecomeFirstResponder: Bool { true }
    var hasText: Bool { true }

    @objc func focus() { becomeFirstResponder() }

    /// Convert the drag to whole lines and report each crossing.
    ///
    /// `setTranslation` puts back only the fractional remainder after each
    /// callback, so a slow drag accumulates towards the next line instead of
    /// being rounded away — the touchscreen equivalent of the Mac's
    /// `wheelTicks`, using the recognizer's own accumulated translation as
    /// the running total instead of a separately stored property.
    @objc private func handlePan(_ recognizer: UIPanGestureRecognizer) {
        guard cellHeight > 0 else { return }
        let translation = recognizer.translation(in: self)
        let lines = Int((translation.y / cellHeight).rounded(.towardZero))
        guard lines != 0 else { return }
        recognizer.setTranslation(
            CGPoint(x: translation.x, y: translation.y - CGFloat(lines) * cellHeight), in: self)
        onScroll?(lines, recognizer.location(in: self))
    }

    func insertText(_ text: String) { onInsertText?(text) }
    func deleteBackward() { onDeleteBackward?() }

    // Every trait below exists to stop iOS from "helping": autocorrect
    // rewriting a flag, smart quotes producing a character no shell
    // recognizes, autocapitalization upper-casing the first letter of a
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
