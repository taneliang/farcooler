import Foundation
import Testing

@testable import Far_Cooler

/// The sidebar has no Orchestrator row (R-14, ov-332): ⌥⌘1 and the title
/// bar's Orchestrator item reveal the chat column, wherever it is.
@MainActor
struct OrchestratorRevealTests {
    typealias Conversation = WorkspaceColumns.Arrangement.Conversation

    @Test("Beside the canvas the chat is already on screen: what's open stays, and a peek over it goes")
    func columnStaysBesideWhatIsOpen() {
        let step = OrchestratorReveal.step(conversation: .column, somethingOpen: true)
        #expect(!step.leavesWhatIsOpen)
        #expect(step.endsPeek)
    }

    @Test("Where the chat is the main area, or kept out of sight, what's open goes so the chat can show",
          arguments: [Conversation.main, .hidden, .none])
    func elsewhereWhatIsOpenGoes(conversation: Conversation) {
        #expect(OrchestratorReveal.step(conversation: conversation, somethingOpen: true).leavesWhatIsOpen)
        #expect(!OrchestratorReveal.step(conversation: conversation, somethingOpen: false).leavesWhatIsOpen)
        #expect(OrchestratorReveal.step(conversation: conversation, somethingOpen: false).endsPeek)
    }

    @Test("Before the window has a width, the old behavior: what's open goes")
    func unknownWidth() {
        #expect(OrchestratorReveal.step(conversation: nil, somethingOpen: true).leavesWhatIsOpen)
    }

    @Test("⌥⌘1 asks for the orchestrator to be selected and given the keyboard, as the title bar's item does")
    func conversationStepSelectsTheOrchestrator() {
        let step = WorkspaceNavigation.boardStep(.conversation, from: .init(opened: true, focus: true))
        #expect(step.selectsOrchestrator)
        #expect(step.keyboard == .conversation)
        #expect(!step.focus, "out of Focus, so the chat is drawn to take it")
    }

    // The title bar's item calls `selectOrchestrator(keyboard: .conversation)` itself, in a closure
    // built inside `ContentView`, which a test can't construct and run without the whole window; the
    // step above and `OrchestratorReveal` are what both entry points then read.
}
