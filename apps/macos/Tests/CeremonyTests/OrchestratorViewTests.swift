import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The orchestrator column's Claude pane and the conversation view (ov-411):
/// the pane gets the same switch, ⌃⌘T and keyboard route as any Claude pane,
/// keeping its draft, and the title bar's orchestrator menu names the switch
/// that's true there.
@MainActor
@Suite(.serialized)
struct OrchestratorViewTests {
    /// The orchestrator's terminal as the CLI lists it: claude, in the main
    /// checkout, with the workspace's seat.
    static func orchestrator(mode: String = "terminal", program: String = "claude") throws -> Terminal {
        try JSONDecoder().decode(
            Terminal.self,
            from: Data(
                #"{"id":"0199bbbb-0000-7000-8000-000000000001","short":"o1","title":"orchestrator","preset":"\#(program)","program":"\#(program)","state":"running","epoch":1,"paneMode":"\#(mode)","role":"orchestrator","workspace":"0199cccc-0000-7000-8000-000000000001"}"#
                    .utf8))
    }

    @Test("The orchestrator's pane is offered the conversation view like any Claude pane")
    func theOrchestratorIsOffered() throws {
        let orchestrator = try Self.orchestrator()
        #expect(orchestrator.isOrchestrator)
        let agents = NativeAgentTests.agents()
        #expect(agents.offers(orchestrator, target: ""))
        #expect(!agents.offers(orchestrator, target: "--host box"), "a runner that isn't this Mac's")
        #expect(!agents.offers(try Self.orchestrator(program: "codex"), target: ""), "a runner that never said codex_view")
        #expect(!agents.offers(try Self.orchestrator(mode: "changes"), target: ""), "a pane drawn by the client")
    }

    @Test("⌃⌘T flips the orchestrator's pane, and a second press flips it back")
    func theKeyFlipsTheOrchestrator() throws {
        let orchestrator = try Self.orchestrator()
        let model = NativeAgentTests.model(orchestrator)
        let agents = NativeAgentTests.agents(model: model)
        defer { NativePaneModel.remember(false, for: orchestrator.id) }
        #expect(agents.toggleView(of: orchestrator, target: ""))
        #expect(model.showsNative)
        #expect(agents.toggleView(of: orchestrator, target: ""))
        #expect(!model.showsNative)
        #expect(!NativeAgentTests.agents(offering: false).toggleView(of: orchestrator, target: ""))
    }

    @Test("The orchestrator's switch keeps its draft and never makes the terminal anew")
    func theSwitchKeepsTheDraft() async throws {
        let orchestrator = try Self.orchestrator()
        let model = NativeAgentTests.model(orchestrator)
        let agents = NativeAgentTests.agents(model: model)
        let life = NativeAgentTests.SurfaceLife()
        let seen = NativeAgentTests.Seen()
        let window = NativeAgentTests.window(
            NativeAgentTests.Probe(
                seen: seen,
                content: NativeSwitch(terminal: orchestrator, target: "", isFocused: true, agents: agents) { focused in
                    NativeAgentTests.StandInSurface(life: life, focused: focused)
                }))
        defer {
            window.close()
            NativePaneModel.remember(false, for: orchestrator.id)
        }
        await NativeAgentTests.settle(window)
        #expect(seen.ids.contains("native-switch"), "the switch is drawn over the orchestrator")
        model.draft = "ask Billing for the totals"
        model.showsNative = true
        await NativeAgentTests.settle(window)
        #expect(life.focused.last == false)
        model.showsNative = false
        await NativeAgentTests.settle(window)
        #expect(life.focused.last == true)
        #expect(life.appeared == 1 && life.disappeared == 0)
        #expect(model.draft == "ask Billing for the totals")
    }

    @Test("The menu names the conversation switch where it's offered, and none where it isn't")
    func theMenuNamesTheRightSwitch() throws {
        #expect(OrchestratorViewSwitch.of(offersConversation: true, conversationShown: false) == .showConversation)
        #expect(OrchestratorViewSwitch.of(offersConversation: true, conversationShown: true) == .showTerminalFromConversation)
        #expect(OrchestratorViewSwitch.of(offersConversation: false, conversationShown: false) == nil)
        #expect(OrchestratorViewSwitch.showConversation.title == "Show Conversation")
        #expect(OrchestratorViewSwitch.showTerminalFromConversation.title == "Show Terminal")
    }
}
