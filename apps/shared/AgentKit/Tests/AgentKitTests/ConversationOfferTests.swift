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

    @Test func aClaudeOrCodexPaneOnAServingRunnerIsOffered() {
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "claude", offered: served) == nil)
        #expect(AgentConversation.unavailable(paneMode: nil, agent: "codex", offered: served) == nil)
    }

    /// The pane first: a shell is never told to turn a setting on.
    @Test func thePaneComesFirst() {
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "shell", offered: nil) == .notAnAgent)
        #expect(AgentConversation.unavailable(paneMode: "agent", agent: "claude", offered: served) == .notAnAgent)
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "claude", running: false, offered: served) == .notRunning)
    }

    @Test func theRunnerSaysWhatsMissing() {
        let off: Set<String> = ["agent_compose", "projector_setting"]
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "claude", offered: nil) == .unreachable)
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "claude", offered: off, projectorOn: false) == .settingOff)
        #expect(
            AgentConversation.unavailable(paneMode: "terminal", agent: "claude", offered: off, projectorOn: nil) == .runnerNeedsUpdate,
            "nothing says the projector is off")
        #expect(AgentConversation.unavailable(paneMode: "terminal", agent: "claude", offered: ["agent_rows"], projectorOn: false) == .runnerNeedsUpdate)
        #expect(
            AgentConversation.unavailable(paneMode: "terminal", agent: "codex", offered: ["agent_rows", "agent_compose"]) == .runnerNeedsUpdate,
            "a runner from before codex's view")
    }

    @Test func eachReasonIsASentenceAndOnlyTheRunnersShowASwitch() {
        let all: [AgentConversation.Unavailable] = [
            .settingOff, .runnerNeedsUpdate, .notAnAgent, .notRunning, .pairingNeeded, .unreachable, .said("Can’t reach it."),
        ]
        for reason in all {
            #expect(reason.sentence.hasSuffix("."), "\(reason)")
            #expect(!reason.sentence.contains("Error"), "\(reason)")
        }
        #expect(AgentConversation.Unavailable.settingOff.sentence == "Turn on Conversation view in Settings.")
        #expect(AgentConversation.Unavailable.notAnAgent.sentence == "Not a Claude or Codex pane.")
        #expect(all.filter(\.isAboutTheRunner).count == 5)
    }
}
