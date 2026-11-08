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

    /// A real wheel event over the view's cell grid.
    @MainActor
    private func wheel(_ view: TerminalRenderView, lines: Int32) -> NSEvent? {
        guard let cg = CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1, wheel1: lines, wheel2: 0, wheel3: 0)
        else { return nil }
        return NSEvent(cgEvent: cg)
    }

    @MainActor
    private func offset(_ view: TerminalRenderView) -> Int { view.core.withSnapshot { $0.displayOffset } ?? -1 }

    @MainActor
    private func filled(_ view: TerminalRenderView, mouseTracking: Bool) {
        view.frame = NSRect(x: 0, y: 0, width: 500, height: 300)
        view.layout()
        for i in 1...200 { view.feed(Array("line \(i)\r\n".utf8)) }
        // What Claude Code sets: report every mouse event, in SGR form.
        if mouseTracking { view.feed(Array("\u{1b}[?1000h\u{1b}[?1006h".utf8)) }
    }

    @MainActor
    @Test("Scrolling a program that tracks the mouse still scrolls local history, and sends nothing")
    func theWheelScrollsWhateverTheAgentTracks() throws {
        let view = TerminalRenderView()
        var sent: [[UInt8]] = []
        view.onInput = { sent.append($0) }
        AgentTerminalCanvas.makeReadOnly(view)
        filled(view, mouseTracking: true)
        view.drainSignals()
        let event = try #require(wheel(view, lines: 3))
        view.scrollWheel(with: event)
        #expect(offset(view) > 0, "the wheel did nothing: it was turned into a mouse report the watcher drops")
        #expect(sent.isEmpty)
    }

    @MainActor
    @Test("The control: an ordinary view hands the wheel to a mouse-tracking program, so the test above can fail")
    func anOrdinaryViewGivesTheWheelToTheProgram() throws {
        let view = TerminalRenderView()
        var sent = 0
        view.onInput = { _ in sent += 1 }
        filled(view, mouseTracking: true)
        view.drainSignals()
        sent = 0
        view.scrollWheel(with: try #require(wheel(view, lines: 3)))
        #expect(offset(view) == 0)
        #expect(sent > 0)
    }

    @MainActor
    @Test("A plain drag selects text in a watching view, whatever the agent tracks")
    func aPlainDragSelects() throws {
        let view = TerminalRenderView()
        AgentTerminalCanvas.makeReadOnly(view)
        filled(view, mouseTracking: true)
        view.drainSignals()
        func click(_ type: NSEvent.EventType, _ x: CGFloat) -> NSEvent? {
            NSEvent.mouseEvent(with: type, location: NSPoint(x: x, y: 150), modifierFlags: [], timestamp: 0,
                windowNumber: 0, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)
        }
        view.mouseDown(with: try #require(click(.leftMouseDown, 5)))
        view.mouseDragged(with: try #require(click(.leftMouseDragged, 60)))
        view.mouseUp(with: try #require(click(.leftMouseUp, 60)))
        view.copy(nil)
        #expect(NSPasteboard.general.string(forType: .string)?.contains("line") == true)
    }

    @MainActor
    @Test("The canvas is labeled for VoiceOver")
    func theCanvasIsLabeled() {
        let view = TerminalRenderView()
        AgentTerminalCanvas.makeReadOnly(view, item: "ov-1")
        #expect(view.accessibilityLabel() == "Agent terminal for ov-1, read-only")
    }

    @Test("A stream that stops says why: finished after a picture, not running when there never was one")
    func theEndIsExplained() {
        #expect(AgentTerminalWatch.status(gotBytes: true, ended: false) == nil)
        #expect(AgentTerminalWatch.status(gotBytes: true, ended: true) == "Finished")
        #expect(AgentTerminalWatch.status(gotBytes: false, ended: true) == "This agent isn’t running right now.")
    }

    @Test("A stream that dropped is not called finished")
    func aDroppedLinkIsNotFinished() {
        #expect(AgentTerminalWatch.status(gotBytes: true, ended: true, exitStatus: 1) == "Lost the connection to this agent.")
    }

    @MainActor
    @Test("A pane wider than the window scrolls sideways at full size instead of being cut off")
    func aWidePaneScrolls() {
        let scroller = AgentTerminalCanvas.Scroller()
        scroller.frame = NSRect(x: 0, y: 0, width: 640, height: 420)
        let view = TerminalRenderView()
        AgentTerminalCanvas.makeReadOnly(view)
        view.trustStreamSizes(true)
        view.reset(columns: 120, rows: 40)
        scroller.terminal = view
        scroller.documentView = view
        scroller.layoutSubtreeIfNeeded()
        scroller.fitDocument()
        #expect(view.frame.width >= view.contentSize.width, "the 120-column screen is clipped")
        #expect(view.frame.width > 640, "nothing to scroll to")
        #expect(view.grid == PaneGrid(columns: 120, rows: 40), "sizing the view changed the agent's grid")
    }
}
