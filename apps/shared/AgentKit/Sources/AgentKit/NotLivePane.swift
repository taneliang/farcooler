import Foundation

// A pane the host says is not running, and the two ways back from it.
//
// `.notLive` is the one terminal phase that is deliberately not retried, and
// that decision is right: it is not a pane that could not be REACHED, it is a
// pane the runner says is not there, and polling one of those every second
// forever would spend a round trip a second to be told the same true thing.
// `TerminalSession.open` says so where it cancels the poller.
//
// What was missing is that the pane can come back. A tmux pane restarted from
// the Mac, an agent relaunched into the same slot, a `starting` pane that had
// not finished starting when this phone happened to look — the fleet poll
// carries the word for all three, every three seconds, and nothing read it. So
// the screen said "Not live" for the life of the process over a pane that had
// been running for an hour, with no button on it either.
//
// The rules are here for `TerminalReach.swift`'s reason: the sentence and the
// decision both live in the iOS target otherwise, which CI compiles and never
// executes.
enum NotLivePane {
    /// Whether the fleet's new word about this pane is worth re-attaching on.
    ///
    /// **An EDGE and not a level, and the caller has to keep it one.** The
    /// answer to "is this pane running" is re-derived on every three-second
    /// poll, so a rule read as a level would re-attach every three seconds for
    /// as long as the host and the FFI disagreed — which is precisely the round
    /// trip a second `.notLive` exists to refuse. Asked of a TRANSITION, it
    /// costs one re-attach per thing that actually happened.
    ///
    /// `starting` counts, and it is not a stretch: a pane whose process is
    /// still coming up is a pane that will be there in a moment, and the
    /// re-attach lands on the screen call that decides. `exited`, `error` and
    /// `lost` do not — those are the states `.notLive` is honestly reporting.
    ///
    /// `unknown` does not either, and that is the conservative direction: it is
    /// a word from a daemon newer than this build, and guessing that an
    /// unreadable state means "running" would re-attach on every state change a
    /// future runner invents.
    static func revives(from was: StateKind, to now: StateKind) -> Bool {
        guard now != was else { return false }
        switch now {
        case .running, .starting: return true
        case .exited, .error, .lost, .unknown: return false
        }
    }

    /// Not a failure and not an alarm: a pane that isn't running is the
    /// ordinary end of a pane.
    static let title = "Not live"

    static func message(for pane: String) -> String {
        "\(pane) has no running pane right now."
    }

    /// The other way back: asking again, by hand, for the case the fleet cannot
    /// cover — a pane restarted under a different id, or a runner too old to
    /// move the state at all.
    ///
    /// "Try Again" and not "Retry": it is what the rest of this app calls the
    /// same move, and it is a sentence rather than a verb stub.
    static let action = "Try Again"
}

/// A pane whose link dropped and is being put back: a wait, not a failure.
///
/// Any ssh hiccup empties the slot a pane polls through, and `Connection`
/// re-dials on its own. Between the two the pane has one sentence to say, and
/// it used to be drawn under a red triangle headed "Could not load", which
/// announced a failure in the middle of a recovery that needed nothing from
/// the person. Red is for a failure they have to act on. The sentence and the
/// decision to read it as a wait live here so `swift test` can pin both; the
/// phone's two screens (a terminal pane and an agent session) and Android's
/// both use the same words.
enum PaneReconnect {
    /// The sentence a dropped link is recorded as.
    static let sentence = "The connection to this runner dropped. Reconnecting…"

    /// The headline of the calm state.
    static let title = "Reconnecting…"

    /// Whether a pane's recorded sentence is this wait rather than a failure.
    static func isReconnecting(_ sentence: String?) -> Bool {
        sentence == Self.sentence
    }

    /// The headline for a pane that could not be loaded: "Couldn’t Load This
    /// Pane", in the case and the contraction every other empty-state title in
    /// the app uses.
    static let paneFailureTitle = "Couldn’t Load This Pane"

    /// The same, for an agent's conversation.
    static let sessionFailureTitle = "Couldn’t Load This Session"

    /// What the core says, as an error's message, when the link is down. A
    /// string because that is all that crosses the FFI on a call.
    static let disconnectedMessage = "not connected"

    /// Whether an error is the link being down rather than the runner refusing.
    static func isDisconnected(_ error: Error) -> Bool {
        error.localizedDescription == disconnectedMessage
    }

    /// What a screen that reads a worktree says when the link is down, in place
    /// of "The request that reads it didn’t finish" over the raw word.
    static let notConnectedSentence = "This runner isn’t connected."
}
