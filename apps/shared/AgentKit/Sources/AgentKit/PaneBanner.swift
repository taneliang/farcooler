import Foundation

/// How a pane's banners are filed with the system's notification center, for
/// the Mac and the phone alike (ov-163, ov-153).
///
/// A banner was posted under `<terminal id>-<activity>`, so an agent that
/// blocked and then finished left two banners, and the comment beside it said
/// the later one replaced the earlier. It does not: the center replaces by
/// request identifier, and the identifiers differed. Nor did anything remove a
/// pane's banners when the pane closed, since `forget` asked for an identifier
/// (`<terminal id>`) that was never posted and, on the phone, had no caller.
///
/// The rule here is one banner per pane: its identifier is the pane's id, so
/// whatever it says next replaces what it said before, and closing it removes
/// that one.
public enum PaneBanner {
    /// The request identifier of every banner about a pane. The thread
    /// identifier is the same id, which is what lets a tap and a foreground
    /// check tell which pane a banner is about.
    public static func identifier(forPane id: String) -> String { id }

    /// What to remove when a pane closes: its identifier, and the three
    /// suffixed ones earlier builds posted, which may still be on a screen.
    public static func removing(pane id: String) -> [String] {
        [id] + ["blocked", "done", "failedRun"].map { "\(id)-\($0)" }
    }

    /// The panes that were there and are not now, in a stable order: the ones
    /// whose banners go.
    public static func closed(before: Set<String>, after: Set<String>) -> [String] {
        before.subtracting(after).sorted()
    }
}
