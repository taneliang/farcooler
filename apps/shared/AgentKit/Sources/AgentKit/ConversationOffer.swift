import Foundation

/// Which pane is offered the conversation view, and the reason a pane isn't,
/// in words a person can act on (ov-443). The Mac and the phone decide by
/// these, so a pane one of them offers the other does too, and a pane
/// neither offers says why on both.
extension AgentConversation {
    /// The agent a pane runs, which the view is offered by: the one the
    /// runner sees running there (`runningAgent`) where it says; else what
    /// the pane was launched as (`program`); else, from a runner too old to
    /// say either, the label.
    ///
    /// Never the label where either is there. Claude names the label after
    /// its session (`Fix the login bug`), so a phone that offered the view
    /// by it never offered it to a claude that had named its session, and a
    /// claude typed into a shell was launched as `shell`.
    ///
    /// An empty `running` is the runner's answer, not a gap: it looked and
    /// nothing runs there, so a pane launched as claude whose claude has
    /// exited is not one (review 1). Only an absent one falls through.
    public static func agent(running: String?, program: String?, preset: String) -> String {
        running ?? program ?? preset
    }

    /// Why a pane isn't offered the conversation view. There is no setting
    /// (ov-830): the view is offered wherever a runner serves it to a
    /// running Claude or Codex pane.
    public enum Unavailable: Equatable, Sendable {
        /// The runner is from before the view, or before this agent's view,
        /// or from before ov-830 with its projector setting off.
        case runnerNeedsUpdate
        /// Not claude or codex in a terminal.
        case notAnAgent
        /// The agent in the pane has exited.
        case notRunning
        /// A remote runner this Mac hasn't paired with, or was unpaired from.
        case pairingNeeded
        /// The runner isn't connected yet, or didn't answer.
        case unreachable
        /// Something only its own words can say: why a runner can't be
        /// reached over ssh, say.
        case said(String)

        /// The reason, in a sentence.
        public var sentence: String {
            switch self {
            case .runnerNeedsUpdate: return "This runner needs an update to show the conversation."
            case .notAnAgent: return "This isn’t a Claude or Codex pane."
            case .notRunning: return "The agent in this pane isn’t running."
            case .pairingNeeded: return "Pairing needed. Pair this runner in Settings, under Devices."
            case .unreachable: return "Far Cooler can’t reach this runner right now."
            case .said(let words): return words
            }
        }

        /// The check that failed, by the name `farcooler terminal
        /// conversation-check` prints for it (ov-830), so a reason read off
        /// a screen finds its line in the command's output.
        public var check: String {
            switch self {
            case .notAnAgent: return "Pane"
            case .notRunning: return "Agent running"
            case .runnerNeedsUpdate: return "Runner version"
            case .pairingNeeded: return "Pairing"
            case .unreachable, .said: return "Connection"
            }
        }

        /// The reason with its check, for a line drawn under a dimmed control,
        /// which is read without hovering.
        public var note: String { "\(check): \(sentence)" }

        /// Whether the pane should still show a dimmed switch that says the
        /// reason: an agent's pane whose runner or pairing is in the way. A
        /// pane that isn't an agent's shows no switch at all.
        public var isAboutTheRunner: Bool {
            switch self {
            case .notAnAgent, .notRunning: return false
            default: return true
            }
        }
    }

    /// Why the pane itself isn't one the view is for, whatever its runner
    /// offers: not claude or codex in a terminal, or not running.
    public static func unavailable(paneMode: String?, agent: String, running: Bool = true) -> Unavailable? {
        guard (paneMode ?? "terminal") == "terminal", agent.hasPrefix("claude") || agent.hasPrefix("codex") else {
            return .notAnAgent
        }
        return running ? nil : .notRunning
    }

    /// Why the pane isn't offered the view, nil where it is.
    ///
    /// The pane first: a shell is never told about its runner. Then the
    /// runner: `offered` is its hello's capabilities, nil while there's no
    /// link to it. A runner without rows and compose needs an update: every
    /// runner from ov-830 offers both, and one from before offers rows only
    /// while its projector setting is on, which an update retires (ov-830:
    /// the app doesn't turn that setting on for it, since that brought back
    /// the always-on projector, and a device without full access can't).
    public static func unavailable(
        paneMode: String?, agent: String, running: Bool = true, offered: Set<String>?
    ) -> Unavailable? {
        if let pane = unavailable(paneMode: paneMode, agent: agent, running: running) { return pane }
        guard let offered else { return .unreachable }
        guard offered.contains(Capability.agentRows.rawValue), offered.contains(Capability.agentCompose.rawValue) else {
            return .runnerNeedsUpdate
        }
        if agent.hasPrefix("codex"), !offered.contains(Capability.codexView.rawValue) { return .runnerNeedsUpdate }
        return nil
    }
}
