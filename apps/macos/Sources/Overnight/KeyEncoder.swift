import AppKit
import Foundation

/// Turns a key event into the exact bytes a terminal program expects.
///
/// This is the whole reason a coding agent works in the app. Claude Code, Codex
/// and any other full-screen TUI read VT sequences: `\u{1b}[A` for cursor up,
/// `0x03` for interrupt, `\u{1b}` alone for escape. Sending human-readable key
/// names would deliver the literal text "Up" instead.
enum KeyEncoder {

    /// Bytes for a key press, or nil when the event carries nothing to send.
    static func bytes(for event: NSEvent, applicationCursorKeys: Bool = false) -> [UInt8]? {
        let flags = event.modifierFlags
        let ctrl = flags.contains(.control)
        let alt = flags.contains(.option)

        // Special keys first: these have no useful character representation.
        if let special = specialKey(event, applicationCursorKeys: applicationCursorKeys) {
            return alt ? [0x1b] + special : special
        }

        guard let chars = event.charactersIgnoringModifiers, !chars.isEmpty else {
            return nil
        }

        // Control chords: Ctrl-A..Ctrl-Z map to 0x01..0x1a, plus the handful of
        // punctuation chords a shell actually uses.
        if ctrl, let scalar = chars.unicodeScalars.first {
            if let c = controlByte(for: scalar) {
                return alt ? [0x1b, c] : [c]
            }
        }

        // Ordinary text. Use `characters` so a composed or shifted key arrives
        // as what the user actually typed.
        let text = event.characters ?? chars
        guard !text.isEmpty else { return nil }
        let payload = Array(text.utf8)

        // Option as Meta: prefix ESC, which is what shells and readline expect.
        return alt ? [0x1b] + payload : payload
    }

    /// Bytes for pasted or programmatically inserted text.
    static func bytes(forText text: String) -> [UInt8] {
        // Normalize newlines to carriage return: a terminal expects CR on Enter.
        Array(text.replacingOccurrences(of: "\n", with: "\r").utf8)
    }

    // MARK: - Detail

    private static func controlByte(for scalar: Unicode.Scalar) -> UInt8? {
        switch scalar {
        case "a"..."z":
            return UInt8(scalar.value - 0x60)
        case "A"..."Z":
            return UInt8(scalar.value - 0x40)
        case "@", " ": return 0x00
        case "[": return 0x1b
        case "\\": return 0x1c
        case "]": return 0x1d
        case "^": return 0x1e
        case "_", "?": return 0x1f
        default: return nil
        }
    }

    private static func specialKey(
        _ event: NSEvent, applicationCursorKeys: Bool
    ) -> [UInt8]? {
        // Cursor keys have two encodings. Normal mode uses CSI, application
        // mode uses SS3, and full-screen programs switch modes deliberately.
        let cursorPrefix: [UInt8] = applicationCursorKeys ? [0x1b, 0x4f] : [0x1b, 0x5b]

        switch event.keyCode {
        case 126: return cursorPrefix + [0x41]  // up
        case 125: return cursorPrefix + [0x42]  // down
        case 124: return cursorPrefix + [0x43]  // right
        case 123: return cursorPrefix + [0x44]  // left
        case 115: return [0x1b, 0x5b, 0x48]  // home
        case 119: return [0x1b, 0x5b, 0x46]  // end
        case 116: return [0x1b, 0x5b, 0x35, 0x7e]  // page up
        case 121: return [0x1b, 0x5b, 0x36, 0x7e]  // page down
        case 117: return [0x1b, 0x5b, 0x33, 0x7e]  // forward delete
        case 36, 76: return [0x0d]  // return / keypad enter -> CR
        case 48: return [0x09]  // tab
        case 51: return [0x7f]  // delete -> DEL, what readline expects
        case 53: return [0x1b]  // escape
        default: return nil
        }
    }

    /// Hex string tmux `send-keys -H` accepts.
    static func hex(_ bytes: [UInt8]) -> String {
        bytes.map { String(format: "%02x", $0) }.joined()
    }
}
