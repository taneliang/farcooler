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
    public static func agent(running: String?, program: String?, preset: String) -> String {
        running ?? program ?? preset
    }

    /// Why a pane isn't offered the conversation view.
    public enum Unavailable: Equatable, Sendable {
        /// The view's setting is off (this Mac's), or the runner's projector
        /// is (a phone's runner).
        case settingOff
        /// The runner is from before the view, or before this agent's view.
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
            case .settingOff: return "Turn on Conversation view in Settings."
            case .runnerNeedsUpdate: return "This runner needs an update to show the conversation."
            case .notAnAgent: return "Not a Claude or Codex pane."
            case .notRunning: return "The agent in this pane isn’t running."
            case .pairingNeeded: return "Pairing needed. Pair this runner in Settings, under Devices."
            case .unreachable: return "Far Cooler can’t reach this runner right now."
            case .said(let words): return words
            }
        }

        /// Whether the pane should still show a dimmed switch that says the
        /// reason: an agent's pane whose runner, setting or pairing is in the
        /// way. A pane that isn't an agent's shows no switch at all.
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
    /// The pane first: a shell is never told to turn a setting on. Then the
    /// runner: `offered` is its hello's capabilities, nil while there's no
    /// link to it; `projectorOn` what it says of its projector, nil where
    /// that's unknown. `agent_rows` is offered only while the projector is
    /// on, so a runner that has the setting and says it's off needs the
    /// setting, and any other runner without rows and compose needs an
    /// update.
    public static func unavailable(
        paneMode: String?, agent: String, running: Bool = true, offered: Set<String>?, projectorOn: Bool? = nil
    ) -> Unavailable? {
        if let pane = unavailable(paneMode: paneMode, agent: agent, running: running) { return pane }
        guard let offered else { return .unreachable }
        guard offered.contains(Capability.agentRows.rawValue), offered.contains(Capability.agentCompose.rawValue) else {
            if projectorOn == false, offered.contains(Capability.projectorSetting.rawValue) { return .settingOff }
            return .runnerNeedsUpdate
        }
        if agent.hasPrefix("codex"), !offered.contains(Capability.codexView.rawValue) { return .runnerNeedsUpdate }
        return nil
    }
}
