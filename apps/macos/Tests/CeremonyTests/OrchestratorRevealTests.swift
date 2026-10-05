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

    @Test("⌥⌘1 and the title bar's Orchestrator item both go through selectOrchestrator, the one place that reveals")
    func oneRoute() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<3 { root.deleteLastPathComponent() }
        let sources = root.appendingPathComponent("Sources/FarCooler")
        func read(_ name: String) throws -> String { try String(contentsOf: sources.appendingPathComponent(name), encoding: .utf8) }
        let toolbar = try read("ContentView+Toolbar.swift")
        let tiling = try read("ContentView+Tiling.swift")
        let detail = try read("ContentView+WorkspaceDetail.swift")
        #expect(toolbar.contains("goToOrchestrator: { selectOrchestrator(keyboard: .conversation) }"))
        #expect(tiling.contains("selectOrchestrator(keyboard: step.keyboard)"))
        #expect(detail.contains("OrchestratorReveal.step("))
        #expect(detail.contains("if reveal.endsPeek { planPeeking = false }"))
    }
}
