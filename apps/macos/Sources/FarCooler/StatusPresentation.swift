import AgentKit
import SwiftUI

/// One status, one presentation (ov-137): the word, the mark and the ink
/// every Mac surface draws a terminal's `Status` with, from this table and
/// no other. A failed turn read "idle" in a task's header and "Idle" on the
/// orchestrator's row, needs-you was the accent in one column and amber in
/// the next, and the board painted a failed agent amber.
///
/// The phones' table is `GlanceState` (ov-125): where a Mac status is one of
/// its five states, it reads the same word in the same tone. The Mac has
/// more states than the wire's three words (a shell that exited, a pane that
/// was lost), and each of those is quiet or a failure here.
///
/// Color only for the states that want a person: amber for needs-you, red
/// for something that went wrong. Everything else is the secondary ink, done
/// included; its ring already says "review".
extension Status {
    /// The phones' state this status reads as, where it is one.
    var glanceState: GlanceState? {
        switch self {
        case .blocked: .needsYou
        case .failedTurn: .failed
        case .done: .finished
        case .working: .working
        case .idle: .unstated
        default: nil
        }
    }

    /// The ink its word takes.
    var tone: GlanceState.Tone {
        switch self {
        case .blocked: .needsYou
        case .failed, .failedRun, .failedTurn, .lost: .failed
        case .starting, .running, .idle, .working, .done, .exited, .unreadable: .quiet
        }
    }

    /// Its word inside a sentence, "claude needs you", or nil where there's
    /// nothing to add to a name: a shell that's merely running, or a runner
    /// that didn't answer.
    var word: String? {
        switch self {
        case .running, .unreadable: nil
        default: label.lowercased()
        }
    }
}

extension GlanceState.Tone {
    /// The ink for words in this tone, in the Mac's window.
    func color(_ scheme: ColorScheme) -> Color {
        switch self {
        case .needsYou: Tint.attention(scheme)
        case .failed: Tint.failure
        case .quiet: .secondary
        }
    }
}
