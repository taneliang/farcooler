import Foundation

/// When a terminal pane reads its claude ask, and when it shows it.
///
/// `TerminalPermissionBar` draws a claude TUI pane's permission ask over the
/// terminal. These are its two decisions, here so `swift test` can reach them.
public enum TerminalAskCard {
    /// Whether the pane's agent stream should be read now.
    ///
    /// Only while the pane is on screen: visible, not under the overview, and
    /// the app active. The stream polls every 700 ms, and a pane nobody can
    /// see has nobody to answer it.
    ///
    /// And while the pane is blocked, OR while an ask it read has not been
    /// resolved yet. The fleet drops `blocked` as soon as claude's dialog
    /// leaves the screen, and the daemon's `Resolved` comes a sample or two
    /// later. A stream stopped on the first would never see the second, and
    /// the pane's next block would show the old ask, whose buttons answer
    /// nothing.
    public static func reads(onScreen: Bool, blocked: Bool, asking: Bool) -> Bool {
        onScreen && (blocked || asking)
    }

    /// Whether the card is drawn for the ask the stream holds.
    ///
    /// Only once the stream has been read since it last started. What it held
    /// before may have been resolved while nothing was reading, and a card
    /// for it would send an answer to an ask that is over.
    public static func shows(asking: Bool, caughtUp: Bool) -> Bool {
        asking && caughtUp
    }
}
