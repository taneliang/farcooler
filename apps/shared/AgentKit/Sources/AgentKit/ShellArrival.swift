import Foundation

// How a pane came to rest, and whether that moves the selected runner — the two
// rules the shell's screen applies on every rest — and the one rule for whether
// a runner is answering.
//
// Here rather than in the screen, for `ShellNavigation.swift`'s reason: the iOS
// target has no unit tests, and a rule a view body decides looks fine in a
// screenshot. See `ShellArrivalTests`.

/// Whether a runner is answering.
enum RunnerLink {
    /// Whether a runner is answering: connected, and its fleet read on this
    /// link. The one rule for every claim about now — a card's marks, a
    /// heading, "Working…" — and Android's `RunnerLink.given`.
    ///
    /// Not `.connected` alone. `start` and `reconnect` set it and then await
    /// the fleet, so for a round trip after every reconnect the fleet on
    /// screen is the last link's, and agents in it may have exited since.
    static func answering(connected: Bool, fleetReadOnThisLink: Bool) -> Bool {
        connected && fleetReadOnThisLink
    }
}

/// How a pane came to rest, as far as the shell can tell.
///
/// The shell knows three of these itself — see `ShellRootView.settle` — and
/// the screen knows the fourth: a rest a deep link asked for looks, from the
/// shell's side, exactly like a move.
enum ShellArrival: Hashable, Sendable {
    /// The first rest of all: wherever the shell was seated when it appeared.
    /// At launch that is the first fleet to ARRIVE, which is a race between
    /// runners and not anybody's choice.
    case appeared
    /// A swipe, or a row chosen in the column.
    case moved
    /// The fleet changed under the shell — a worktree vanished, a runner
    /// answered — and the shell was re-seated onto what was left.
    case reseated
    /// A tapped notification or Live Activity card, honored.
    case linked
}

/// Whether a rest should move `RunnerStore.selected` onto the runner it
/// arrived on.
///
/// The selection is what `RunnerStore.selected` persists, and the runner menu
/// that used to set it is gone — so with every runner connected it follows the
/// runner you are on.
///
/// **Only a deliberate move is followed.** It used to follow every rest, and
/// the first rest at launch is wherever the shell was seated — the first
/// runner whose fleet ARRIVED. One launch where the selected runner was slow
/// wrote the fast one down as the choice, and every launch after that landed
/// on it. A re-seat and a deep link are not choices either. What changes the
/// selection besides a move is explicit and does not come through here.
///
/// Never with one runner at a time, where the selection is which runner is
/// CONNECTED: a rest on the runner being switched away from, mid-switch,
/// would switch straight back.
enum ShellSelection {
    static func follows(
        _ arrival: ShellArrival, everyRunnerAtOnce: Bool,
        arrived: String, selected: String?
    ) -> Bool {
        arrival == .moved && everyRunnerAtOnce && arrived != selected
    }
}
