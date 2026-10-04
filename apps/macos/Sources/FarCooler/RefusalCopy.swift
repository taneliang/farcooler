import AgentKit
import Foundation

/// The Mac's way into AgentKit's `RunnerRefusal` (ov-160).
///
/// A runner's refusal reaches the Mac as the CLI's stderr with a `code:` line
/// (`--json`), and the word on it is the only thing to switch on. Seven tables
/// here once spelled their own English for the same words, so the same refusal
/// read differently by where it came from, and `scope-denied` sent a start to
/// "Check that it’s reachable" about a runner that had answered. Every table
/// now asks `RunnerRefusal` first, and keeps its own sentence only for what
/// that enum has no word for or for a refusal it can say more about from the
/// request (the `what:` line, a subject's name).
enum RefusalCopy {
    /// The shared refusal this stderr names, or nil for no `code:` line or a
    /// word `RunnerRefusal` has no sentence for.
    static func refusal(in message: String?) -> RunnerRefusal? {
        TaskFailure.code(in: message).flatMap(RunnerRefusal.init(rawValue:))
    }

    /// `context`, then the shared sentence for the word, or nil.
    ///
    /// The context says what didn't happen, for a screen that names its
    /// subject; the sentence says why and what to do, once, for every screen.
    static func sentence(after context: String, _ message: String?) -> String? {
        refusal(in: message).map { "\(context) \($0.sentence)" }
    }
}

extension TaskFailure {
    /// Whether the runner refused for want of a confirmation: a dirty
    /// worktree removed without its name, or a turn in flight. By the word, not
    /// by the CLI's English containing "confirmation".
    static func isConfirmationRequired(_ message: String?) -> Bool {
        code(in: message) == "confirmation-required"
    }
}
