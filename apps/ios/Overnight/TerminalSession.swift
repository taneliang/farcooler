import Foundation
import OvernightVT
import SwiftUI

/// One terminal's screen, kept live while a `TerminalView` is on screen.
///
/// The wire carries a full screen, not a diff — `terminal.screen` is a
/// snapshot, the way `fleet` is a snapshot. So each poll gets a fresh
/// `VTCore` rather than feeding into the last one: a persistent core would
/// need the dump to be self-clearing to stay correct, and "assume every
/// capture starts by wiping the grid" is a fact about the host's tmux version,
/// not something this file should have to trust. Recreating is one allocation
/// a second and buys freedom from that assumption entirely.
///
/// This never decides whether a terminal is "live" — the host does, the same
/// way `Connection` never computes a workspace's state. An unreadable screen
/// is reported exactly as the host phrased it.
@MainActor
final class TerminalSession: ObservableObject {
    enum Phase: Equatable {
        case connecting
        case notLive
        case failed(String)
        case live
    }

    @Published private(set) var phase: Phase = .connecting
    @Published private(set) var grid: TerminalGrid?

    private var terminalID: String
    private let core: ClientCore
    private var vt: VTCore?
    private var poller: Task<Void, Never>?
    private var lastRequestedSize: (columns: Int, rows: Int)?

    /// The last screen decoded, kept so the next poll can tell whether
    /// anything actually changed. See `poll()` for what that buys.
    private var lastScreen: ScreenResponse?

    // MARK: - Adaptive polling
    //
    // NOT streaming. The daemon has no push channel for a terminal's screen —
    // that is protocol work being done separately — so "adaptive" here means
    // choosing how often to ask, cheaply, rather than changing what is asked
    // for. A capture costs the host about 16ms; at a fixed 1000ms interval
    // that 16ms was rounding error next to 984ms of the phone doing nothing
    // but waiting for its own clock. Polling faster while the screen is
    // actually moving, and backing off once it stops, spends that budget
    // where a human would notice it — mid-keystroke — instead of evenly
    // wherever a plain interval happens to land.

    /// The busiest a poll loop gets: about as fast as a human can perceive a
    /// screen redrawing, and comfortably above the host's own ~16ms capture
    /// cost — polling faster than the capture itself would just queue up SSH
    /// round trips the host cannot answer any quicker.
    private static let fastInterval: Double = 0.1
    /// The idle cadence a quiet terminal settles into. Still fast enough that
    /// "nothing happened in the last second" reads as current, not stale.
    private static let slowInterval: Double = 1.0
    /// How quickly an unchanging screen backs off. Geometric rather than a
    /// fixed step, so a screen that goes quiet coasts to the slow interval in
    /// a handful of polls rather than taking as long to slow down as it took
    /// to speed up.
    private static let backoffFactor: Double = 1.6

    private var interval: Double = TerminalSession.fastInterval

    init(terminalID: String, core: ClientCore) {
        self.terminalID = terminalID
        self.core = core
    }

    deinit { poller?.cancel() }

    /// Point this same session at a different terminal — what the tab strip
    /// calls when you tap a sibling.
    ///
    /// A new `TerminalSession` per tap would work too, but it would tear down
    /// and restart the poller `Task` for no reason: the loop already just
    /// asks for whatever `terminalID` currently is on every tick, so
    /// retargeting it here is enough. The screen is cleared immediately
    /// rather than left showing the outgoing terminal until the next tick,
    /// which would make the tap look like it did nothing. `lastScreen` is
    /// cleared for the same reason `grid` is: it describes the terminal being
    /// left, and comparing the new one's first capture against it would read
    /// as "unchanged" by coincidence and back off a screen nobody has shown
    /// yet.
    func switchTo(_ id: String) {
        guard id != terminalID else { return }
        terminalID = id
        vt = nil
        grid = nil
        lastScreen = nil
        phase = .connecting
        wake()
    }

    /// Reflow the pane to the size this screen would like.
    ///
    /// Separate from opening the terminal, and that separation is the point: a
    /// pane belongs to whoever else is looking at it too, so this is something
    /// you ask for rather than something that happens because you tapped a row.
    func fitPaneToViewport() async {
        guard let size = lastRequestedSize else { return }
        _ = try? await core.call(
            "terminal.resize",
            ["terminal": terminalID, "columns": size.columns, "rows": size.rows])
        // The resize itself changes the screen, but not in a way this
        // terminal has captured yet — without clearing it, a resize to
        // exactly the content already on screen (rare, but possible) would
        // compare equal and the redraw would wait for the next backed-off
        // tick instead of showing the new dimensions immediately.
        lastScreen = nil
        wake()
    }

    /// Start polling. Deliberately does NOT resize the pane.
    ///
    /// It used to, and that was wrong in a way only visible with two clients: a
    /// pane is not private to whoever is looking at it. Opening a terminal on the
    /// phone reflowed the shared window down to phone width — 81 columns became
    /// 43 — so every pane in that layout was squeezed on the Mac at the same
    /// moment, and stayed squeezed after the phone was put down. A viewer must
    /// not reshape what it is viewing.
    ///
    /// So the phone renders the grid the host has, scaled to fit. A wide pane on
    /// a narrow screen is small, which is honest; the alternative — everyone
    /// else's layout collapsing because someone glanced at their phone — is not
    /// a trade worth making.
    ///
    /// The size is still accepted here because the view knows it, and a future
    /// version can use it to offer a deliberate "resize this pane to my screen"
    /// — an explicit act, not a side effect of looking.
    func configure(columns: Int, rows: Int) async {
        guard columns > 0, rows > 0 else { return }
        lastRequestedSize = (columns, rows)
        guard poller == nil else { return }
        startLoop()
    }

    /// Stop polling. Called from `.onDisappear` — a backgrounded terminal view
    /// has no business spending the host's SSH round trips or this phone's
    /// battery on a screen nobody is reading.
    func stop() {
        poller?.cancel()
        poller = nil
    }

    /// Cancel and restart the loop at the fast interval — "poll right now,
    /// then resume at full speed" — without ending up with two loops running.
    ///
    /// A plain `Task { await poll() }` fired alongside the existing loop would
    /// work for the one poll, but the existing loop's own `Task.sleep` would
    /// still be counting down on the OLD interval underneath it, so the next
    /// scheduled tick would still land late. Restarting the loop is what
    /// actually changes the cadence rather than just sneaking in one extra
    /// poll ahead of it.
    private func wake() {
        interval = Self.fastInterval
        poller?.cancel()
        startLoop()
    }

    private func startLoop() {
        poller = Task { [weak self] in
            while let self, !Task.isCancelled {
                await self.poll()
                guard !Task.isCancelled else { return }
                try? await Task.sleep(for: .seconds(self.interval))
            }
        }
    }

    private func poll() async {
        do {
            let data = try await core.call("terminal.screen", ["terminal": terminalID])
            let response = try JSONDecoder().decode(ScreenResponse.self, from: data)

            // The cheap compare that makes backing off free: `ScreenResponse`
            // carries the still-base64 payload, so this is a byte compare of
            // exactly what the host sent, before any of the more expensive
            // work — base64 decoding, feeding a fresh `VTCore`, copying a
            // snapshot into cells SwiftUI will diff — runs on a screen that
            // has not moved. Most polls of an idle terminal end right here.
            guard response != lastScreen else {
                interval = min(interval * Self.backoffFactor, Self.slowInterval)
                return
            }
            lastScreen = response
            interval = Self.fastInterval

            guard let bytes = Data(base64Encoded: response.contents) else {
                phase = .failed("The host sent a screen this device could not decode.")
                return
            }

            let core = VTCore(columns: response.columns, rows: response.rows)
            core.feed([UInt8](bytes))
            vt = core
            grid = core.withSnapshot {
                TerminalGrid(
                    snapshot: $0,
                    cursorRow: response.cursorRow,
                    cursorColumn: response.cursorColumn)
            }
            phase = .live
        } catch {
            // "resource not found" is the host's answer for a terminal that is
            // not currently a live pane — restarted, stopped, never started.
            // Everything else is a real failure, and the two must read
            // differently: one is "come back later", the other is "something
            // is wrong".
            let message = error.localizedDescription
            phase = message == "resource not found" ? .notLive : .failed(message)
            // Back off on a persistent error too — a terminal that keeps
            // failing to load has no "changing" to detect, and retrying it
            // every 100ms would hammer a host that has already said no.
            interval = min(interval * Self.backoffFactor, Self.slowInterval)
        }
    }

    /// Send typed text. Each scalar is encoded on its own so a Ctrl modifier —
    /// which only ever applies to the next key — transforms exactly one of
    /// them, matching how a physical Ctrl key behaves.
    func send(text: String, modifiers: VTModifiers) async {
        guard let vt else { return }
        var bytes: [UInt8] = []
        for (index, scalar) in text.unicodeScalars.enumerated() {
            bytes += vt.encode(scalar: scalar, modifiers: index == 0 ? modifiers : [])
        }
        await write(bytes)
    }

    func send(key: UInt32, modifiers: VTModifiers = []) async {
        guard let vt else { return }
        await write(vt.encode(key: key, modifiers: modifiers))
    }

    private func write(_ bytes: [UInt8]) async {
        guard !bytes.isEmpty else { return }
        let hex = bytes.map { String(format: "%02x", $0) }.joined()
        _ = try? await core.call("terminal.write", ["terminal": terminalID, "hex": hex])
        // The moment a key is sent is the moment staleness is least
        // acceptable — the user is looking right at this screen waiting to
        // see what they typed. Snap back to the fast interval and poll now
        // rather than waiting out however long a quiet screen had backed off
        // to.
        wake()
    }
}

private struct ScreenResponse: Decodable, Equatable {
    var contents: String
    var columns: Int
    var rows: Int
    var cursorColumn: Int
    var cursorRow: Int
}

/// A plain-data copy of one screen, safe to publish and hold past the moment
/// it was read.
///
/// `VTSnapshot` borrows a buffer that dies on the next core call; a `@Published`
/// property lives until the next poll overwrites it and is read by SwiftUI on
/// its own schedule, not this file's. Those two lifetimes cannot share a
/// pointer, so this copies once, here, and nowhere downstream has to worry
/// about it again.
struct TerminalGrid {
    var columns: Int
    var rows: Int
    var cells: [TerminalCell]
    var cursorRow: Int
    var cursorColumn: Int

    init(snapshot: VTSnapshot, cursorRow: Int, cursorColumn: Int) {
        columns = snapshot.columns
        rows = snapshot.rows
        cells = (0..<(snapshot.rows * snapshot.columns)).map { i in
            TerminalCell(snapshot.cells[i])
        }
        // Clamp rather than trust: the cursor position comes from a separate
        // call (`terminal.cursor`, folded into this response by the host) than
        // the screen dump, and the two can race a resize by one poll interval.
        self.cursorRow = min(max(cursorRow, 0), max(rows - 1, 0))
        self.cursorColumn = min(max(cursorColumn, 0), max(columns - 1, 0))
    }

    subscript(row: Int, column: Int) -> TerminalCell {
        cells[row * columns + column]
    }
}

struct TerminalCell {
    var character: Character?
    var bold: Bool
    var wide: Bool
    /// Already resolved for `OVERNIGHT_VT_FLAG_INVERSE` — the core reports
    /// foreground and background separately from the flag, and a renderer
    /// that forgot to swap them would draw reverse-video text invisibly on
    /// itself.
    var foreground: Color
    var background: Color

    init(_ cell: OvernightVtCell) {
        character = cell.character
        bold = cell.isBold
        wide = cell.isWide
        foreground = Color(packed: cell.isInverse ? cell.bg : cell.fg)
        background = Color(packed: cell.isInverse ? cell.fg : cell.bg)
    }
}

extension Color {
    init(packed: UInt32) {
        self.init(
            red: Double((packed >> 16) & 0xFF) / 255,
            green: Double((packed >> 8) & 0xFF) / 255,
            blue: Double(packed & 0xFF) / 255)
    }
}

/// Colours the core did not resolve, because they belong to the phone's
/// screen, not to any program running on the host: the fill behind a short
/// last row and the cursor block. Values mirror the Mac app's so the same
/// terminal looks like the same terminal on both.
enum TerminalPalette {
    static let backgroundPacked: UInt32 = 0x12_14_19
    static let background = Color(packed: backgroundPacked)
    static let cursor = Color(red: 0.44, green: 0.66, blue: 1.0, opacity: 0.9)
}
