import Foundation

/// Why a pane in agent mode has no agent in it, and what a phone says about it.
///
/// **The runner sends a stable machine word; this file owns the sentence.** The
/// word leaves the shim on the daemon link, crosses the protocol on
/// `Terminal.agent_failure`, and arrives here as `Terminal.agentFailure` — the
/// same rule `TunnelError.code` follows for the tunnel and `AdapterTestOutcome`
/// follows for the Test button. A Rust error string must never reach a screen,
/// which is exactly what happened while the only report of a failed adapter was
/// a line printed to a pane's stdout that the transcript view then covered up.
///
/// **Why the copy lives in AgentKit rather than in `AgentView.body`.** The iOS
/// UI suite is compiled by CI and never executed, so a sentence composed inside
/// a `View.body` is a sentence nothing reads back — the same argument that put
/// `ShellNavigation`, `ShellFlight` and `AgentCardRows` here. `swift test
/// --package-path apps/shared/AgentKit` runs on every push, and it is the only
/// place an iOS-facing rule about these words actually runs.
///
/// The pane STAYS a chat. Nothing here flips a pane back to a terminal:
/// respawning it automatically would race whatever the reader was in the middle
/// of typing. `AgentFailureCopy.action` names the switch and leaves pressing it
/// to them.
public enum AgentFailure: String, CaseIterable, Sendable {
    /// Nothing on the runner is configured to speak ACP for this agent. The fix
    /// is a `config.toml` entry.
    case noAdapter = "no-adapter"
    /// The adapter refused for want of credentials. The fix is not in this app
    /// at all: it is the agent's own login, run on the runner.
    case notAuthenticated = "not-authenticated"
    /// It started and then said nothing until the runner gave up waiting.
    case adapterSilent = "adapter-silent"
    /// Everything else: it would not spawn, it hung up, or it refused for a
    /// reason of its own.
    case adapterFailed = "adapter-failed"
}

/// What a chat with no agent in it says on a phone.
///
/// A headline, a sentence under it, and the label on the one action offered —
/// which is the composition every full-screen state in both phone apps already
/// uses (`AgentView.status`, Android's `AgentEmptyState`). Chosen here so both
/// phones choose it the same way and so a test can read it back.
public struct AgentFailureCopy: Equatable, Sendable {
    /// The headline. One line, in this app's own voice.
    public let title: String
    /// What to do about it. Never nil, because the reader of a pane that
    /// silently spun for a minute is owed a next step even when the honest one
    /// is only "go and look at what it printed".
    public let message: String
    /// The label on the way out.
    ///
    /// The words already on the pane's overflow item, deliberately: this offers
    /// the same switch, and one thing with two names is two things to a reader.
    /// Title case, because it is a button.
    public let action: String

    /// The copy for a word off the wire, or nil when nothing has said this pane
    /// failed.
    ///
    /// **An absent word is not a failure.** A pane still coming up looks
    /// exactly like this, so nil here leaves the caller drawing the spinner and
    /// the "still starting" ladder it already had.
    ///
    /// **An unrecognized word IS a failure.** This is the one place these apps
    /// deliberately part company with the Mac, which reads a fifth word as no
    /// failure at all. A newer runner only ever sends this field to say a pane
    /// gave up; reading its word as silence puts the endless spinner back —
    /// which is the whole bug — so a word this build has never heard of reads
    /// as the generic failure, with the offer to go and look at the terminal.
    /// That is precisely what `adapter-failed` already means.
    public static func forWord(_ word: String?) -> AgentFailureCopy? {
        guard let word, !word.isEmpty else { return nil }
        return copy(for: AgentFailure(rawValue: word) ?? .adapterFailed)
    }

    /// The sentence for a word this build knows.
    ///
    /// Each of the four gets its own, because each has a different fix — a
    /// config entry, a login on the runner, waiting, and nobody knows. One
    /// shared "the agent could not start" would be the endless spinner again
    /// with better manners.
    public static func copy(for failure: AgentFailure) -> AgentFailureCopy {
        switch failure {
        case .noAdapter:
            AgentFailureCopy(
                title: "No chat adapter for this agent",
                message:
                    "Nothing on the runner is set up to talk to it. Add an adapter for it in the "
                    + "runner's config.toml, then switch this pane back to a chat.",
                action: showTheTerminal)
        case .notAuthenticated:
            AgentFailureCopy(
                title: "This agent needs you to sign in",
                message:
                    "Sign in with the agent's own command on the runner, then switch this pane "
                    + "back to a chat.",
                action: showTheTerminal)
        case .adapterSilent:
            AgentFailureCopy(
                title: "The agent started but never answered",
                message:
                    "It may still be installing. This pane's terminal has whatever it printed.",
                action: showTheTerminal)
        case .adapterFailed:
            AgentFailureCopy(
                title: "The agent couldn't start",
                message:
                    "Nothing here knows why. This pane's terminal has whatever it printed.",
                action: showTheTerminal)
        }
    }

    /// One label for one action, spelled once.
    ///
    /// Byte for byte the overflow item in `ShellPaneBar.paneModeItem` and in
    /// Android's `TerminalPane`, because it does the same thing to the same
    /// pane.
    private static let showTheTerminal = "Show the Terminal"
}
