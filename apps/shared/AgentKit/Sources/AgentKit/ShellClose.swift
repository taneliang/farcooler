import Foundation

/// Closing a terminal: what it costs, and what a phone has to say first.
///
/// **The rule and the words, in one place, because both phones ship them.**
/// Closing is two calls — `terminal.stop` then `terminal.remove` — and the
/// daemon refuses the second one on a live pane
/// (`Service::remove_terminal` answers `RunningProcesses` for `Running` and
/// `Starting`). So the stop is not a courtesy, it is what makes the removal
/// legal, and a phone that only stopped would leave the dead rectangle
/// `remain-on-exit` deliberately keeps.
///
/// **A stop is not undoable and nothing here pretends otherwise.** The pane is
/// killed and the record is deleted; there is no bin to fish it out of. That is
/// what earns the confirmation, and it is also why the confirmation is only for
/// the case that has something to lose: a pane whose process has already exited
/// has no agent to interrupt, so asking about it would be a tax charged on the
/// harmless case to protect the rare one.
///
/// **Why the copy is here rather than in a `View.body`.** It is the one part of
/// this that can be wrong without anybody noticing — a duration that says `0s`,
/// an agent named twice, a sentence that claims something is running when it is
/// not — and a sentence assembled inside a SwiftUI body is a sentence no test
/// can read back. `ShellCloseTests` reads all of it. Android says the same
/// three sentences from `model/ShellClose.kt`, which is a port of this file and
/// is tested the same way; the two phones may differ in GESTURE and must not
/// differ in what they tell somebody they are about to lose.
enum ShellClose {
    /// The sheet a running pane earns, or nil for one that has already exited.
    struct Question: Equatable {
        /// Names the pane, because the swipe that got here named nothing: a
        /// row slid sideways and a red button appeared under a thumb.
        var title: String
        /// Names the agent and how long it has been going, then says what
        /// closing does and that it cannot be taken back.
        var message: String
    }

    /// The button that does it. Title case, and it says the noun: a bare
    /// `Close` in a dialog raised by a swipe is a word with nothing attached
    /// to it.
    static let confirm = "Close Terminal"

    /// Whether closing this pane must be confirmed first.
    ///
    /// **The same two states `remove_terminal` refuses**, and that is the
    /// whole definition rather than a coincidence worth restating: the
    /// confirmation exists because the close has to STOP something, and the
    /// only panes it has to stop are the ones the daemon will not remove
    /// while they live. `Lost`, `Exited`, `Error` and `Unknown` have no
    /// process to interrupt.
    static func mustAsk(about terminal: Terminal) -> Bool {
        switch StateKind.parse(terminal.state) {
        case .running, .starting: return true
        case .exited, .error, .lost, .unknown: return false
        }
    }

    /// What to ask, or nil when there is nothing to ask about.
    ///
    /// Nil is not "no opinion" — it is the answer that means CLOSE IT NOW, and
    /// the caller is expected to read it that way. Returning a question with an
    /// empty body instead would put a sheet in front of every close, which is
    /// exactly the tax the ruling refused.
    ///
    /// `now` is an argument for `Terminal.displayDuration(at:)`'s reason: a
    /// `Date()` read inside is a value nothing can observe and nothing can
    /// test.
    static func question(about terminal: Terminal, at now: Date) -> Question? {
        guard mustAsk(about: terminal) else { return nil }
        return Question(
            title: "Close “\(terminal.label)”?",
            message: "\(running(terminal, at: now)) Closing stops it and removes the tab. "
                + "There’s no undo.")
    }

    /// The first sentence: what is in this pane, and how long it has been at it.
    ///
    /// Three shapes, and the difference between them is what the runner has
    /// actually said. `displayDuration` already picks the honest clock for each
    /// state — the TURN's for a working agent, the STATE's for a blocked one —
    /// and answers nil both when the host never sent a timestamp and when the
    /// answer would be under five seconds. Nil there means the sentence loses
    /// its clause rather than gaining a "0s", because "working for 0s" is the
    /// one thing worse than not saying.
    private static func running(_ terminal: Terminal, at now: Date) -> String {
        let name = Terminal.name(of: terminal.preset)
        guard let doing = verb(terminal.agent), let elapsed = terminal.displayDuration(at: now)
        else {
            return "It’s running \(name)."
        }
        return "It’s running \(name), which has been \(doing) for \(elapsed)."
    }

    /// How to say what the agent is doing, for the two states with a clock.
    ///
    /// Only `working` and `blocked`, and deliberately the same pair
    /// `Terminal.statusDuration(at:)` answers for: an idle agent has been idle
    /// since some moment nobody is interested in, and `done` is idle that
    /// nobody has read. Neither is a thing you interrupt, so neither gets a
    /// clause claiming it was.
    ///
    /// "Waiting on you" rather than `AgentActivity.label`'s "Needs you". The
    /// label is a badge — a noun phrase for a chip — and this is the middle of
    /// a sentence.
    private static func verb(_ activity: AgentActivity) -> String? {
        switch activity {
        case .working: return "working"
        case .blocked: return "waiting on you"
        case .none, .idle, .done, .unknown: return nil
        }
    }
}
