import Foundation
import FarCoolerVT

/// Swift's view of the Rust terminal core.
///
/// A thin, safe wrapper: it owns the handle's lifetime, converts between Swift
/// and C types, and nothing else. Every decision about what bytes mean — what
/// color an escape sequence produces, what an arrow key encodes to under the
/// program's current mode — lives on the other side of this boundary. That is
/// what let the Mac app's renderer and this one answer to the same handle
/// without agreeing on a single line of emulator logic between them.
///
/// Not thread-safe by design — the core is confined to the main actor, which
/// is where drawing happens anyway, and is the same discipline the header
/// documents for every platform.
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

    func feed(_ bytes: [UInt8]) {
        guard let handle, !bytes.isEmpty else { return }
        bytes.withUnsafeBufferPointer { farcooler_vt_feed(handle, $0.baseAddress, $0.count) }
    }

    func resize(columns: Int, rows: Int) {
        guard let handle else { return }
        farcooler_vt_resize(handle, UInt16(clamping: columns), UInt16(clamping: rows))
    }

    /// Scroll the view. Positive goes back into history.
    func scroll(lines: Int32) {
        guard let handle else { return }
        farcooler_vt_scroll(handle, lines)
    }

    /// Jump back to the live screen. Call this on input: typing into a
    /// scrolled-back view would show the user nothing of what they typed.
    func scrollToBottom() {
        guard let handle else { return }
        farcooler_vt_scroll_to_bottom(handle)
    }

    /// Read the screen and hand it to `body`.
    ///
    /// Scoped rather than returned: the cell buffer belongs to the core and is
    /// only valid until the next call on this handle. Handing it out directly
    /// would invite a dangling read the moment a poll fed new bytes; a closure
    /// makes the lifetime the compiler's problem instead of a bug report.
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
                cursorVisible: raw.cursor_visible
            )
        )
    }

    /// Encode a keystroke for the program currently running.
    ///
    /// The core answers because the answer depends on modes it holds: an arrow
    /// key is different bytes under application cursor mode, and a Ctrl
    /// modifier turns a printable scalar into a control code (Ctrl-C is `c`
    /// with `.control`, not a separate key). Neither is this file's to decide.
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
}

/// A borrowed view of the screen. Valid only inside `withSnapshot`.
struct VTSnapshot {
    let cells: UnsafeBufferPointer<FarCoolerVtCell>
    let columns: Int
    let rows: Int
    /// Where the emulator itself has the caret, after every byte it has been
    /// given.
    ///
    /// Worth taking over the host's answer to `terminal.cursor` whenever there
    /// is one: that is a second round trip whose answer is true at a different
    /// instant than the screen it is drawn on, so a fast-moving caret is drawn
    /// a poll behind the text it is supposed to be sitting in. This one cannot
    /// disagree with the screen beside it, because the same bytes put both
    /// where they are.
    let cursorRow: Int
    let cursorColumn: Int
    let cursorVisible: Bool

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
    var isInverse: Bool { flags & UInt16(FARCOOLER_VT_FLAG_INVERSE) != 0 }
    /// A double-width character. The next column is its spacer; a renderer
    /// that draws both would show the glyph twice.
    var isWide: Bool { flags & UInt16(FARCOOLER_VT_FLAG_WIDE) != 0 }

    /// Nil for a blank cell, so a renderer can skip drawing rather than
    /// measuring and painting an empty glyph four thousand times a frame.
    var character: Character? {
        guard let scalar = Unicode.Scalar(ch), scalar != " " else { return nil }
        return Character(scalar)
    }
}
