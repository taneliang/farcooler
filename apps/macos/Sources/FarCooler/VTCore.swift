import CFarCoolerVT
import Foundation

/// Swift's view of the Rust terminal core.
///
/// A thin, safe wrapper: it owns the handle's lifetime, converts between Swift
/// and C types, and nothing else. Every decision about what bytes mean lives on
/// the other side of this boundary, which is what lets iOS and Android share it.
///
/// Not thread-safe by design — the core is confined to the main actor, which is
/// where drawing happens anyway.
@MainActor
final class VTCore {
    /// The pointer is just an address — it carries no isolated state of its
    /// own. The discipline that matters is that only the main actor ever calls
    /// through it, and deinit is by definition the last thing that does.
    nonisolated(unsafe) private var handle: UnsafeMutableRawPointer?

    init(columns: Int, rows: Int) {
        handle = farcooler_vt_new(UInt16(clamping: columns), UInt16(clamping: rows))
    }

    deinit {
        if let handle { farcooler_vt_free(handle) }
    }

    // MARK: - Output

    func feed(_ bytes: [UInt8]) {
        guard let handle, !bytes.isEmpty else { return }
        bytes.withUnsafeBufferPointer { farcooler_vt_feed(handle, $0.baseAddress, $0.count) }
    }

    /// Recolour every cell the next snapshot produces.
    ///
    /// `colors` is nineteen packed values: sixteen ANSI, then foreground,
    /// background, cursor. The core resolves cell colours when a snapshot is
    /// taken rather than when bytes arrive, so this recolours scrollback too —
    /// which is what makes switching themes instant instead of a repaint of
    /// history the terminal no longer holds.
    @discardableResult
    func setPalette(_ colors: [UInt32]) -> Bool {
        guard let handle else { return false }
        return colors.withUnsafeBufferPointer {
            farcooler_vt_set_palette(handle, $0.baseAddress, $0.count)
        }
    }

    func resize(columns: Int, rows: Int) {
        guard let handle else { return }
        farcooler_vt_resize(handle, UInt16(clamping: columns), UInt16(clamping: rows))
    }

    /// Changes whenever the screen may have changed. An unchanged value means
    /// the frame can be skipped entirely.
    var revision: UInt64 {
        guard let handle else { return 0 }
        return farcooler_vt_revision(handle)
    }

    var isAlternateScreen: Bool {
        guard let handle else { return false }
        return farcooler_vt_alt_screen(handle)
    }

    /// Release a synchronized update the program never closed.
    ///
    /// A full-screen program wraps an atomic repaint in `\e[?2026h` …
    /// `\e[?2026l` so it is never drawn half-finished, and the core holds those
    /// bytes back until the closing sequence — which is what keeps a clear and
    /// its redraw from being sampled as a blank frame. When the program is
    /// killed between the two, or the link drops, that sequence never arrives.
    ///
    /// So this is the deadline, and it has to be driven by something that ticks,
    /// because the case it exists for is the one where no more bytes are coming.
    /// The display link is the only thing here that ticks regardless. It costs
    /// one comparison in the core when nothing is held, which is nearly always.
    @discardableResult
    func flushExpiredSync() -> Bool {
        guard let handle else { return false }
        return farcooler_vt_flush_sync(handle)
    }

    /// Scroll the view. Positive goes back into history.
    func scroll(lines: Int32) {
        guard let handle else { return }
        farcooler_vt_scroll(handle, lines)
    }

    /// Return to the live screen, but only if we had left it.
    ///
    /// Guarded so that ordinary typing does not bump the revision counter and
    /// force a redraw of a screen that has not changed.
    func scrollToBottomIfScrolled() {
        guard let handle else { return }
        var raw = FarCoolerVtSnapshot()
        guard farcooler_vt_snapshot(handle, &raw), raw.display_offset > 0 else { return }
        farcooler_vt_scroll_to_bottom(handle)
    }

    /// Read the screen and draw it inside `body`.
    ///
    /// Scoped rather than returned, because the cell buffer belongs to the core
    /// and is only valid until the next call. Handing it out would invite a
    /// dangling read; a closure makes the lifetime the compiler's problem.
    func withSnapshot<T>(_ body: (VTSnapshot) -> T) -> T? {
        guard let handle else { return nil }
        var raw = FarCoolerVtSnapshot()
        guard farcooler_vt_snapshot(handle, &raw), let cells = raw.cells else { return nil }
        let count = Int(raw.rows) * Int(raw.columns)
        return body(
            VTSnapshot(
                cells: UnsafeBufferPointer(start: cells, count: count),
                columns: Int(raw.columns),
                rows: Int(raw.rows),
                cursorRow: Int(raw.cursor_row),
                cursorColumn: Int(raw.cursor_column),
                cursorVisible: raw.cursor_visible,
                displayOffset: Int(raw.display_offset),
                historySize: Int(raw.history_size)
            )
        )
    }

    /// Bytes the program wants written back, such as a cursor-position reply.
    ///
    /// These must reach the program or a full-screen agent waits forever for an
    /// answer that never comes.
    func takePendingWrites() -> [UInt8] {
        guard let handle else { return [] }
        var out: [UInt8] = []
        var buffer = [UInt8](repeating: 0, count: 256)
        while true {
            let n = buffer.withUnsafeMutableBufferPointer {
                farcooler_vt_take_writes(handle, $0.baseAddress, $0.count)
            }
            guard n > 0 else { break }
            out.append(contentsOf: buffer[0..<n])
            if n < buffer.count { break }
        }
        return out
    }

    func takeBell() -> Bool {
        guard let handle else { return false }
        return farcooler_vt_take_bell(handle)
    }

    /// Text the program asked to put on the clipboard (OSC 52), or nil.
    ///
    /// Two calls, like `encode(paste:)` and for the mirror of its reason: the
    /// core writes nothing when the buffer is short rather than truncating,
    /// because half a copied command is worse than no copy at all.
    ///
    /// There is no counterpart that READS the clipboard. A program asking for
    /// its contents is refused inside the core — see `farcooler_vt_take_clipboard`.
    func takeClipboard() -> String? {
        guard let handle else { return nil }
        let needed = farcooler_vt_take_clipboard(handle, nil, 0)
        guard needed > 0 else { return nil }

        var buffer = [UInt8](repeating: 0, count: needed)
        let written = buffer.withUnsafeMutableBufferPointer {
            farcooler_vt_take_clipboard(handle, $0.baseAddress, $0.count)
        }
        guard written == needed else { return nil }
        return String(decoding: buffer, as: UTF8.self)
    }

    /// The URL under a cell, with where it sits so it can be underlined.
    ///
    /// The core decides what counts as a URL and which schemes may be opened —
    /// terminal output is not trusted input, and putting that list here would
    /// make it three lists across three platforms.
    func url(atRow row: Int, column: Int) -> (url: String, span: FarCoolerVtUrlSpan)? {
        guard let handle, row >= 0, column >= 0 else { return nil }
        var span = FarCoolerVtUrlSpan()
        let needed = farcooler_vt_url_at(
            handle, UInt16(clamping: row), UInt16(clamping: column), &span, nil, 0)
        guard needed > 0 else { return nil }

        var buffer = [UInt8](repeating: 0, count: needed)
        let written = buffer.withUnsafeMutableBufferPointer {
            farcooler_vt_url_at(
                handle, UInt16(clamping: row), UInt16(clamping: column), &span,
                $0.baseAddress, $0.count)
        }
        guard written == needed else { return nil }
        return (String(decoding: buffer, as: UTF8.self), span)
    }

    var title: String? {
        guard let handle, let ptr = farcooler_vt_title(handle) else { return nil }
        return String(cString: ptr)
    }

    // MARK: - Input

    /// Encode a keystroke for the program currently running.
    ///
    /// The core answers because the answer depends on modes it holds: an arrow
    /// key is different bytes under application cursor mode, and this view has
    /// no way to know that.
    func encode(key: UInt32, modifiers: VTModifiers) -> [UInt8] {
        guard let handle else { return [] }
        var buffer = [UInt8](repeating: 0, count: 16)
        let n = buffer.withUnsafeMutableBufferPointer {
            farcooler_vt_encode_key(handle, key, modifiers.rawValue, $0.baseAddress, $0.count)
        }
        return Array(buffer[0..<n])
    }

    func encode(scalar: Unicode.Scalar, modifiers: VTModifiers) -> [UInt8] {
        encode(key: scalar.value, modifiers: modifiers)
    }

    /// Encode a mouse event, or nil when the program does not want it — in
    /// which case the event is ours to handle locally.
    func encode(
        mouse button: UInt32,
        action: UInt32,
        column: Int,
        row: Int,
        modifiers: VTModifiers
    ) -> [UInt8]? {
        guard let handle else { return nil }
        var buffer = [UInt8](repeating: 0, count: 32)
        let n = buffer.withUnsafeMutableBufferPointer {
            farcooler_vt_encode_mouse(
                handle,
                button,
                action,
                UInt16(clamping: column),
                UInt16(clamping: row),
                modifiers.rawValue,
                $0.baseAddress,
                $0.count
            )
        }
        return n > 0 ? Array(buffer[0..<n]) : nil
    }

    /// Encode a paste, bracketed if the program asked for that.
    func encode(paste text: String) -> [UInt8] {
        guard let handle else { return [] }
        var input = Array(text.utf8)
        // Ask for the size first: a paste is arbitrarily long and the core
        // refuses to truncate one silently.
        let needed = input.withUnsafeMutableBufferPointer {
            farcooler_vt_encode_paste(handle, $0.baseAddress, $0.count, nil, 0)
        }
        guard needed > 0 else { return [] }

        var buffer = [UInt8](repeating: 0, count: needed)
        let written = input.withUnsafeMutableBufferPointer { src in
            buffer.withUnsafeMutableBufferPointer { dst in
                farcooler_vt_encode_paste(
                    handle, src.baseAddress, src.count, dst.baseAddress, dst.count)
            }
        }
        return Array(buffer[0..<written])
    }
}

/// A borrowed view of the screen. Valid only inside `withSnapshot`.
struct VTSnapshot {
    let cells: UnsafeBufferPointer<FarCoolerVtCell>
    let columns: Int
    let rows: Int
    let cursorRow: Int
    let cursorColumn: Int
    let cursorVisible: Bool
    /// How far back the view is scrolled, in lines. Zero means live.
    let displayOffset: Int
    /// Lines available above the screen.
    let historySize: Int

    subscript(row: Int, column: Int) -> FarCoolerVtCell {
        cells[row * columns + column]
    }
}

struct VTModifiers: OptionSet {
    let rawValue: UInt32
    static let shift = VTModifiers(rawValue: UInt32(FARCOOLER_VT_MOD_SHIFT))
    static let alt = VTModifiers(rawValue: UInt32(FARCOOLER_VT_MOD_ALT))
    static let control = VTModifiers(rawValue: UInt32(FARCOOLER_VT_MOD_CTRL))
}

extension FarCoolerVtCell {
    var isBold: Bool { flags & UInt16(FARCOOLER_VT_FLAG_BOLD) != 0 }
    var isItalic: Bool { flags & UInt16(FARCOOLER_VT_FLAG_ITALIC) != 0 }
    var isUnderlined: Bool { flags & UInt16(FARCOOLER_VT_FLAG_UNDERLINE) != 0 }
    var isInverse: Bool { flags & UInt16(FARCOOLER_VT_FLAG_INVERSE) != 0 }
    /// A double-width character. The next column is its spacer; skip it.
    var isWide: Bool { flags & UInt16(FARCOOLER_VT_FLAG_WIDE) != 0 }

    var character: Character? {
        guard let scalar = Unicode.Scalar(ch), scalar != " " else { return nil }
        return Character(scalar)
    }
}
