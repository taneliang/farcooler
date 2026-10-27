import Foundation
import Testing

@testable import AgentKit

// ov-443: which pane is offered the conversation view, and why not.

@Suite struct ConversationOfferTests {
    private let served: Set<String> = ["agent_rows", "agent_compose", "projector_setting", "codex_view"]

    /// The owner's panes: a claude typed into a shell, and one that named its
    /// session. Neither was offered the view by its label.
    @Test func theAgentIsTheOneRunningThenWhatWasLaunched() {
        #expect(AgentConversation.agent(running: "claude", program: "shell", preset: "Fix the login bug") == "claude")
        #expect(AgentConversation.agent(running: nil, program: "claude", preset: "Fix the login bug") == "claude")
        #expect(AgentConversation.agent(running: nil, program: nil, preset: "claude") == "claude", "a runner too old to say")
        #expect(AgentConversation.agent(running: nil, program: "shell", preset: "claude") == "shell", "never the label where program is there")
    }

    /// A runner that looked and found a shell says so with an empty agent. That
    /// beats the preset the pane was launched as: a claude that quit is no claude.
    @Test func aRunnerThatSaysNothingRunsIsBelievedOverWhatWasLaunched() {
        let agent = AgentConversation.agent(running: "", program: "claude", preset: "claude")
        #expect(agent == "")
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: agent, offered: served) == .notAnAgent)
    }

    /// Each reason names the check `farcooler terminal conversation-check`
    /// prints, and its note leads with it (ov-830).
    @Test func eachReasonNamesItsCheck() {
        #expect(AgentConversation.Unavailable.runnerNeedsUpdate.check == "Runner version")
        #expect(AgentConversation.Unavailable.pairingNeeded.check == "Pairing")
        #expect(AgentConversation.Unavailable.said("Host key changed.").check == "Connection")
        #expect(AgentConversation.Unavailable.unreachable.note == "Connection: Far Cooler can’t reach this runner right now.")
        // Not "Pairing: Pairing needed." (ov-830 review, S3).
        #expect(AgentConversation.Unavailable.pairingNeeded.note == "Pairing: Pair this runner in Settings, under Devices, to read its conversations here.")
    }

    @Test func aClaudeOrCodexPaneOnAServingRunnerIsOffered() {
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "claude", offered: served) == nil)
        #expect(AgentConversation.unavailable(paneMode: nil, agent: "codex", offered: served) == nil)
    }

    /// The pane first: a shell is never told about its runner.
    @Test func thePaneComesFirst() {
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "shell", offered: nil) == .notAnAgent)
        #expect(AgentConversation.unavailable(paneMode: "changes", agent: "claude", offered: served) == .notAnAgent)
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "claude", running: false, offered: served) == .notRunning)
    }

    /// A runner from before ov-830 with its projector setting off offers
    /// the setting and compose but not rows: it needs an update, which is
    /// true, and nothing tells a person to find a setting that's gone.
    @Test func theRunnerSaysWhatsMissing() {
        let off: Set<String> = ["agent_compose", "projector_setting"]
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "claude", offered: nil) == .unreachable)
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "claude", offered: off) == .runnerNeedsUpdate)
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "claude", offered: ["agent_rows"]) == .runnerNeedsUpdate)
        #expect(
            AgentConversation.unavailable(paneMode: "terminal", agent: "codex", offered: ["agent_rows", "agent_compose"]) == .runnerNeedsUpdate,
            "a runner from before codex's view")
    }

    @Test func eachReasonIsASentenceAndOnlyTheRunnersShowASwitch() {
        let all: [AgentConversation.Unavailable] = [
            .runnerNeedsUpdate, .notAnAgent, .notRunning, .pairingNeeded, .unreachable, .said("Can’t reach it."),
        ]
        for reason in all {
            #expect(reason.sentence.hasSuffix("."), "\(reason)")
            #expect(!reason.sentence.contains("Error"), "\(reason)")
        }
        #expect(AgentConversation.Unavailable.notAnAgent.sentence == "This isn’t a Claude or Codex pane.")
        #expect(all.filter(\.isAboutTheRunner).count == 4)
    }
}
