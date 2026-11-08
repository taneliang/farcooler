import AppKit
import Testing

@testable import Far_Cooler

/// Watching an engine agent's terminal reaches it with nothing (ov-972).
struct AgentTerminalWatchTests {
    /// A program asking where the cursor is: an emulator answers it by writing to the pane.
    private let question: [UInt8] = Array("\u{1b}[6n".utf8)

    @MainActor
    @Test("An ordinary terminal answers a program's question, so the check below can fail")
    func anOrdinaryTerminalAnswers() {
        let view = TerminalRenderView()
        var sent = 0
        view.onInput = { _ in sent += 1 }
        view.feed(question)
        view.drainSignals()
        #expect(sent > 0, "the control: this view writes back, or the read-only test proves nothing")
    }

    @MainActor
    @Test("A watching view sends the agent nothing, takes no keyboard and reports no size")
    func aWatchingViewSendsNothing() {
        let view = TerminalRenderView()
        var sent = 0
        view.onInput = { _ in sent += 1 }
        view.onGeometry = { _, _ in sent += 1 }
        AgentTerminalCanvas.makeReadOnly(view)
        view.feed(question)
        view.drainSignals()
        view.frame = NSRect(x: 0, y: 0, width: 500, height: 300)
        view.layout()
        #expect(sent == 0, "something left a watching view")
        #expect(view.onInput == nil)
        #expect(view.onGeometry == nil)
        #expect(!view.acceptsFirstResponder)
    }

    @Test("The only thing the view ever asks the CLI to run is a watch")
    func theCommandIsAWatch() {
        #expect(AgentTerminalCanvas.command(workspace: "w", item: "ov-1") == ["workflow", "watch", "w", "ov-1"])
    }
}
