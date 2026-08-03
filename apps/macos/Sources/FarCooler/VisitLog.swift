import Foundation

/// When you were last in each terminal.
///
/// The switcher orders by this rather than by agent activity, because the two
/// answer different questions. `activitySince` says when an agent last changed
/// what it was doing — useful, but it means a busy agent you have never opened
/// outranks the pane you were in ten seconds ago. What ⌘P is for is getting back,
/// and "back" is about where YOU have been.
///
/// It is Alt-Tab's rule exactly: the pane you are in now is not the first entry,
/// because you are already there. The most recent one you are NOT in comes first,
/// so ⌘P then Return returns you to where you just were, and holding the panel
/// open walks further back. That single property is what makes it possible to
/// work two agents against each other without ever aiming at a tile.
///
/// Kept in memory. Persisting would make the first ⌘P of a session order itself
/// by a session you no longer remember, which is worse than falling back to
/// agent activity for panes you have not visited yet.
@MainActor
final class VisitLog: ObservableObject {
    static let shared = VisitLog()

    /// Terminal id to a monotonic counter.
    ///
    /// A counter, not a clock: two visits inside the same millisecond have to
    /// order, and the wall clock can go backwards.
    private var visits: [String: UInt64] = [:]
    private var next: UInt64 = 1

    /// The terminal being looked at, which is the one Alt-Tab skips.
    private(set) var current: String?

    func visited(_ terminal: String) {
        guard current != terminal else { return }
        current = terminal
        visits[terminal] = next
        next += 1
    }

    /// A terminal that has gone takes its history with it, or a reused id would
    /// inherit a position it never earned.
    func forget(_ terminal: String) {
        visits.removeValue(forKey: terminal)
        if current == terminal { current = nil }
    }

    func rank(_ terminal: String) -> UInt64 { visits[terminal] ?? 0 }

    func isCurrent(_ terminal: String) -> Bool { current == terminal }
}
