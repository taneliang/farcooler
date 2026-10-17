import Foundation

/// What a Paste on a terminal pane sends (ov-604).
///
/// A phone has no other way to get a long string into a pane, and the one that
/// matters most is the sign-in code Claude asks for: `code#state`, about a
/// hundred characters. Typed, it is a hundred keystrokes the program may act on
/// one at a time. A paste is one write, bracketed when the program asked for
/// that, with a trailing newline left inside it. It never presses Return.
///
/// The bracketing itself is the shared core's (`encode_paste`, the same call
/// Android's pane makes), so the bytes match; what lives here is the decision
/// that a paste happens at all, and that it goes out whole.
public enum TerminalPaste {
    /// The one write for this clipboard text, or nil when nothing should go.
    ///
    /// - Parameters:
    ///   - text: the clipboard's text, if it has any.
    ///   - mayType: whether this phone's grant lets it write to the pane at all
    ///     (`terminal.write` takes Control scope). Without it nothing is sent
    ///     and nothing is encoded.
    ///   - encode: the core's paste encoder for the program running in the pane.
    public static func bytes(
        for text: String?, mayType: Bool, encode: (String) -> [UInt8]
    ) -> [UInt8]? {
        guard mayType, let text, !text.isEmpty else { return nil }
        let bytes = encode(text)
        return bytes.isEmpty ? nil : bytes
    }
}
