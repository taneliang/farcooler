import Testing

@testable import Far_Cooler

/// New agent panes on the Mac open in the terminal (ov-361, R-27).
struct AgentOpeningTests {
    /// A Mac with no stored choice never switches a fresh agent pane to chat.
    @Test func aNewPaneOnAFreshMacStaysInTheTerminal() {
        #expect(AgentOpening.preferChatDefault == false)
        #expect(
            !AgentOpening.opensAsChat(
                preferChat: AgentOpening.preferChatDefault, canSwitch: true, isAgentPane: false,
                alreadyOffered: false))
    }

    /// Someone who turned the preference on still gets chat for a new pane.
    @Test func theOptInStillOpensAChat() {
        #expect(
            AgentOpening.opensAsChat(
                preferChat: true, canSwitch: true, isAgentPane: false, alreadyOffered: false))
    }

    /// A pane remembered in agent mode is left there, and one already offered
    /// the chat isn't dragged back into it.
    @Test func aRememberedModeIsRespected() {
        #expect(
            !AgentOpening.opensAsChat(
                preferChat: true, canSwitch: true, isAgentPane: true, alreadyOffered: false))
        #expect(
            !AgentOpening.opensAsChat(
                preferChat: true, canSwitch: true, isAgentPane: false, alreadyOffered: true))
        #expect(
            !AgentOpening.opensAsChat(
                preferChat: true, canSwitch: false, isAgentPane: false, alreadyOffered: false))
    }
}
