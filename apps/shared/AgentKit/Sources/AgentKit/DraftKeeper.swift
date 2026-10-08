import Foundation

/// The conversation composer's unsent text, kept on disk per terminal (ov-369
/// review F4, R-38), so app quit, jetsam or process death doesn't lose it.
///
/// Bring Here clears claude's own box once its text is in the composer, which
/// leaves the composer's in-memory draft as the only copy. This keeps a second
/// one. Text only: images aren't persisted. One `UserDefaults` key per
/// terminal, so a pane's draft is read without decoding anyone else's.
public enum NativeDraftStore {
    /// The most a saved draft holds: 64 KB of UTF-8. A longer draft is cut at a
    /// character boundary, so what comes back is a prefix of what was typed.
    public static let capBytes = 64 * 1024

    static func key(_ terminal: String) -> String { "nativeDraft.\(terminal)" }

    /// The terminal's saved draft, or "" for none.
    public static func read(_ terminal: String, in defaults: UserDefaults = .standard) -> String {
        defaults.string(forKey: key(terminal)) ?? ""
    }

    /// Save the terminal's draft. An empty one removes the key, so a pane that
    /// has sent everything costs nothing on disk.
    public static func write(_ text: String, for terminal: String, in defaults: UserDefaults = .standard) {
        if text.isEmpty {
            defaults.removeObject(forKey: key(terminal))
        } else {
            defaults.set(capped(text), forKey: key(terminal))
        }
    }

    /// `text` cut to at most `capBytes` of UTF-8, never inside a character.
    public static func capped(_ text: String) -> String {
        guard text.utf8.count > capBytes else { return text }
        var end = text.startIndex
        var used = 0
        for index in text.indices {
            let size = text[index].utf8.count
            if used + size > capBytes { break }
            used += size
            end = text.index(after: index)
        }
        return String(text[..<end])
    }
}

/// Saves one terminal's composer draft as it changes, and gives it back when
/// the pane's model is made.
///
/// A change is written after a quiet moment (about 300 ms), so typing costs one
/// write and not one per key. An empty draft is written at once and cancels
/// anything waiting: a confirmed send must not have a late write bring the sent
/// text back.
@MainActor
public final class DraftKeeper {
    private let terminal: String
    private let defaults: UserDefaults
    private let delay: Duration
    private var pending: Task<Void, Never>?

    public init(terminal: String, defaults: UserDefaults = .standard, delay: Duration = .milliseconds(300)) {
        self.terminal = terminal
        self.defaults = defaults
        self.delay = delay
    }

    /// What was saved: the composer's text when the pane's model is made.
    public var restored: String { NativeDraftStore.read(terminal, in: defaults) }

    /// The composer's text changed to `text`.
    public func changed(_ text: String) {
        pending?.cancel()
        pending = nil
        if text.isEmpty {
            NativeDraftStore.write("", for: terminal, in: defaults)
            return
        }
        let (terminal, defaults, delay) = (terminal, defaults, delay)
        pending = Task { @MainActor in
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled else { return }
            NativeDraftStore.write(text, for: terminal, in: defaults)
        }
    }

    /// Returns once any waiting write has landed. For tests, and for a
    /// harness that needs the draft on disk.
    public func settled() async { await pending?.value }
}
