import Foundation

/// What a terminal with no running pane says, and the two ways off it (ov-191).
///
/// Shared by the Mac and the phone so the sentence and the offer are one
/// decision. A lost terminal used to be a row on the Mac that nothing happened
/// on when clicked, and a "Not live" page on the phone with only Try Again.
/// Both now open a page that says why and offers Restart and, for a lost one,
/// Dismiss.
///
/// **What Restart brings back.** The daemon's `restart_terminal` runs the
/// terminal's preset again: an agent preset comes back as that agent, back in
/// its conversation when there is one to reopen, and a shell comes back as a
/// shell. The preset is recorded when a terminal is created, so every terminal
/// has one. What a person typed into a shell isn't recorded anywhere, so a
/// shell's Restart says so before it's pressed, not after.
public enum LostPane {
    /// The states this page is for. Anything else has a running pane, or
    /// may have one, and gets the terminal itself.
    public enum Kind: Equatable, Sendable {
        /// No pane claims it: the pane was closed outside Far Cooler, tmux
        /// was quit, or the runner restarted.
        case lost
        /// Its program ended and the pane is gone.
        case exited
        /// It couldn't be started.
        case error

        /// The daemon's word for a state, in either case: the Mac's CLI
        /// writes `LOST` and the phone's core `lost`.
        public init?(state: String) {
            switch state.lowercased() {
            case "lost": self = .lost
            case "exited": self = .exited
            case "error": self = .error
            default: return nil
            }
        }
    }

    public enum Action: Equatable, Sendable {
        case restart
        case dismiss
    }

    /// What the page offers. Dismiss is for a lost terminal alone, since
    /// that's the only state `terminal.dismiss_lost` accepts. The others
    /// keep a pane of their own, and the runner refuses to dismiss them.
    public static func actions(for kind: Kind) -> [Action] {
        switch kind {
        case .lost: return [.restart, .dismiss]
        case .exited, .error: return [.restart]
        }
    }

    public static func title(for kind: Kind) -> String {
        switch kind {
        case .lost: return "Terminal Lost"
        case .exited: return "Terminal Ended"
        case .error: return "Terminal Didn’t Start"
        }
    }

    /// Why there's nothing running, in words. For a lost terminal, all three
    /// causes, because the runner can't tell them apart: it only knows that
    /// no tmux pane claims this terminal any more.
    public static func explanation(for kind: Kind) -> String {
        switch kind {
        case .lost:
            return "Its tmux pane is gone. The pane was closed outside Far Cooler, tmux was quit, "
                + "or the runner restarted. Nothing is running in it now."
        case .exited:
            return "Its program ended. Nothing is running in it now."
        case .error:
            return "The runner couldn’t start it."
        }
    }

    /// What Restart will bring back, said before it's pressed.
    ///
    /// `preset` is the terminal's `command_preset`, which may carry a model
    /// (`claude:opus`). An empty one is a shell, as the daemon reads it.
    public static func restartNote(preset: String) -> String {
        let program = preset.split(separator: ":", maxSplits: 1).first.map(String.init) ?? ""
        switch program {
        case "", "shell":
            return "Restart opens a new shell in this worktree. What was running in it wasn’t "
                + "recorded, so you’ll need to start that again."
        case "claude":
            return "Restart opens Claude Code again in this worktree, back in its conversation if it had one."
        case "codex":
            return "Restart opens Codex again in this worktree, back in its conversation if it had one."
        case "cursor":
            return "Restart opens Cursor again in this worktree."
        default:
            return "Restart runs \(program) again in this worktree."
        }
    }

    /// What Dismiss does, beside its button.
    public static let dismissNote = "Dismiss removes it from the list."
}
