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

    private let terminalID: String
    private let core: ClientCore
    private var vt: VTCore?
    private var poller: Task<Void, Never>?
    private var lastRequestedSize: (columns: Int, rows: Int)?

    init(terminalID: String, core: ClientCore) {
        self.terminalID = terminalID
        self.core = core
    }

    deinit { poller?.cancel() }

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
        poller = Task { [weak self] in
            while !Task.isCancelled {
                await self?.poll()
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    /// Stop polling. Called from `.onDisappear` — a backgrounded terminal view
    /// has no business spending the host's SSH round trips or this phone's
    /// battery on a screen nobody is reading.
    func stop() {
        poller?.cancel()
        poller = nil
    }

    private func poll() async {
        do {
            let data = try await core.call("terminal.screen", ["terminal": terminalID])
            let response = try JSONDecoder().decode(ScreenResponse.self, from: data)
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
    }
}

private struct ScreenResponse: Decodable {
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
