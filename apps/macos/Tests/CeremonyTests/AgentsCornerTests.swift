import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The agents in the orchestrator's corner (ov-829), drawn in real offscreen
/// windows: the control and the view switch one group in from the paper's
/// top trailing corner, on the orchestrator's pane alone; the chat's paper
/// starting on the line of the paper beside it; the first row clear of the
/// corner; and Queued with its Send Now on one baseline.
@MainActor
@Suite(.serialized)
struct AgentsCornerTests {
    typealias T = NativeAgentTests
    typealias P = NativePolishTests
    typealias W = WorkspaceAgentsTests

    static func agents() throws -> RunningAgents {
        MacAgents.running(workspace: W.main, host: "", fleet: try W.busy())
    }

    /// A pane as `TerminalPane` draws one: its switch over a stand-in
    /// terminal, on the paper, in the canvas's inset.
    static func pane(_ terminal: Terminal, agents: NativeAgents, corner: PaneCornerAccessory?) -> some View {
        NativeSwitch(terminal: terminal, target: "", isFocused: true, agents: agents) { _ in Color.black }
            .paneCard()
            .probed("paper")
            .paneCanvas()
            .environment(\.paneCornerAccessory, corner)
    }

    @Test("On the orchestrator's pane the agents sit beside the switch, one height, in from the paper's corner by the corner inset")
    func theCornerGroup() async throws {
        let terminal = try T.terminal()
        let model = T.model(terminal)
        let native = T.agents(model: model)
        let agents = try Self.agents()
        let corner = PaneCornerAccessory(terminal: terminal.id, content: AnyView(AgentsCornerButton(agents: agents) { _ in }))
        let (frames, window) = await W.drawn(Self.pane(terminal, agents: native, corner: corner), width: 640, height: 400)
        defer {
            window.close()
            NativePaneModel.remember(false, for: terminal.id)
        }
        let paper = try #require(frames.views["paper"], "\(frames.views.keys.sorted())")
        let control = try #require(frames.views["agents-corner"], "\(frames.views.keys.sorted())")
        let toggle = try #require(frames.views["native-switch"])
        #expect(abs(toggle.maxX - (paper.maxX - PaneCorner.inset)) < 0.5, "the switch in from the trailing edge: \(toggle) \(paper)")
        #expect(abs(control.minY - (paper.minY + PaneCorner.inset)) < 0.5, "in from the top edge: \(control) \(paper)")
        #expect(abs(control.minY - toggle.minY) < 0.5 && abs(control.height - toggle.height) < 0.5, "one row: \(control) \(toggle)")
        #expect(abs(control.height - PaneCorner.control) < 0.5)
        #expect(abs(toggle.minX - control.maxX - Spacing.tight) < 0.5, "beside it: \(control) \(toggle)")
    }

    @Test("Another pane keeps its own switch alone: the agents are the orchestrator's")
    func onlyTheOrchestratorsPane() async throws {
        let terminal = try T.terminal()
        let model = T.model(terminal)
        let native = T.agents(model: model)
        let corner = PaneCornerAccessory(terminal: "someone-else", content: AnyView(AgentsCornerButton(agents: try Self.agents()) { _ in }))
        let (frames, window) = await W.drawn(Self.pane(terminal, agents: native, corner: corner), width: 640, height: 400)
        defer {
            window.close()
            NativePaneModel.remember(false, for: terminal.id)
        }
        #expect(frames.views["agents-corner"] == nil)
        #expect(frames.views["native-switch"] != nil)
    }

    @Test("The control says the count in words, and who needs you to VoiceOver")
    func words() throws {
        let agents = try Self.agents()
        #expect(agents.summary == "5 agents")
        #expect(AgentsCornerButton.value(.empty) == "")
        let waiting = GlanceMark(attention: .needsYou, core: .atAPrompt)
        func pane(_ id: String, _ mark: GlanceMark) -> AgentPane {
            AgentPane(terminal: id, worktree: id, label: id, isOrchestrator: false, workspace: W.main, activity: "", startedMs: nil, mark: mark, subagents: [], subagentIds: [], subagentsRunning: nil)
        }
        #expect(AgentsCornerButton.value(RunningAgents.workspace(W.main, runner: "", panes: [pane("a", waiting)])) == "1 needs you")
        #expect(AgentsCornerButton.value(RunningAgents.workspace(W.main, runner: "", panes: [pane("a", waiting), pane("b", waiting)])) == "2 need you")
    }

    // MARK: - The column

    /// A workspace with the canvas beside the chat: what's opened under a
    /// header-tall jump bar, or the plan's card, and the chat's paper.
    static func workspace(opened: String?) -> some View {
        WorkspaceView(
            opened: opened, hasConversation: true, hasBoard: false, cell: WorkspaceColumns.defaultCell, focused: false,
            navigatorWidth: .constant(320),
            conversation: { Color.white.paneCard().probed("chat-paper").paneCanvas() },
            navigator: { Color.clear },
            breadcrumb: { _ in Color.clear.columnHeader() },
            detail: { _, _ in Color.white.paneCard().probed("opened-paper").paneCanvas() },
            motion: .linear(duration: 0), swap: nil, split: true,
            // `contentCard()`, with the probe on its paper.
            home: { AnyView(Color.white.frame(maxWidth: .infinity, maxHeight: .infinity).paneCard().probed("home-paper").paneCanvas()) })
    }

    @Test("Beside something opened under its jump bar, the chat's paper starts on its paper's line; beside the plan, on the plan's")
    func thePapersLineUp() async throws {
        let width = WorkspaceColumns.canvasFoldsBelow + 40
        let (opened, openedWindow) = await W.drawn(Self.workspace(opened: "shell"), width: width, height: 500)
        defer { openedWindow.close() }
        let chat = try #require(opened.views["chat-paper"], "\(opened.views.keys.sorted())")
        let beside = try #require(opened.views["opened-paper"])
        #expect(abs(chat.minY - beside.minY) < 0.5, "one line: chat \(chat.minY), opened \(beside.minY)")
        #expect(beside.minY > ColumnHeader.height, "under the jump bar: \(beside)")

        let (home, homeWindow) = await W.drawn(Self.workspace(opened: nil), width: width, height: 500)
        defer { homeWindow.close() }
        let chatHome = try #require(home.views["chat-paper"])
        let plan = try #require(home.views["home-paper"], "\(home.views.keys.sorted())")
        #expect(abs(chatHome.minY - plan.minY) < 0.5, "one line: chat \(chatHome.minY), plan \(plan.minY)")
    }

    // MARK: - The conversation under it

    @Test("A queued message says Queued and Send Now on one baseline, a group's gap under its bubble")
    func queuedOnOneBaseline() async throws {
        let view = QueuedLine(text: "Also run the iOS UI tests for the band before you hand it back.", sendNow: {})
            .probed("queued").padding(20)
        let (frames, window) = await W.drawn(view, width: 520, height: 200)
        defer { window.close() }
        let label = try #require(frames.views["native-queued-label-baseline"], "\(frames.views.keys.sorted())")
        let send = try #require(frames.views["native-send-now-baseline"])
        #expect(abs(label.minY - send.minY) < 0.5, "one baseline: \(label.minY) \(send.minY)")
        #expect(send.minX > label.minX, "Send Now after Queued")
    }

    @Test("Scrolled to its top, the conversation's first row starts clear of the corner's controls")
    func firstRowClearOfTheCorner() async throws {
        let model = T.model(try T.terminal())
        model.store.apply(try await model.store.ledger.page(T.page(NativeAgentSubagentTests.rows(many: true))))
        let (frames, window) = await W.drawn(NativeAgentView(model: model, isFocused: false, showTerminal: {}), width: 640, height: 300)
        defer { window.close() }
        // It opens at its newest; go to the oldest, as a person scrolling up would.
        func scrollers(_ view: NSView) -> [NSScrollView] { (view as? NSScrollView).map { [$0] } ?? [] + view.subviews.flatMap(scrollers) }
        let scroller = try #require(window.contentView.flatMap(scrollers)?.max { $0.frame.height < $1.frame.height })
        // As far up as it goes, its top margin included: the clip view keeps
        // it in bounds. Again until it stays there: the list follows its end
        // while its rows land, which can take it back down under load.
        var far = scroller.contentView.bounds
        far.origin.y = -10_000
        let top = scroller.contentView.constrainBoundsRect(far).origin
        for _ in 0..<20 {
            scroller.contentView.scroll(to: top)
            scroller.reflectScrolledClipView(scroller.contentView)
            await NativeAgentTests.settle(window, still: frames) // capture-exempt: a layout settle after a scroll, not a capture
            if abs(scroller.contentView.bounds.origin.y - top.y) < 0.5 { break }
        }
        #expect(abs(scroller.contentView.bounds.origin.y - top.y) < 0.5, "scrolled to the top")
        let transcript = try #require(frames.views["native-transcript"], "\(frames.views.keys.sorted())")
        let rows = frames.views.filter { $0.key.hasPrefix("native-row") }.values.map(\.minY).filter { $0 >= transcript.minY - 1 }
        let first = try #require(rows.min(), "\(frames.views.keys.sorted())")
        #expect(first - transcript.minY >= PaneCorner.inset + PaneCorner.control, "the first row \(first - transcript.minY) pt down, the corner's foot at \(PaneCorner.inset + PaneCorner.control)")
        #expect(first - transcript.minY <= PaneCorner.clearance + TranscriptLayout.padding + 1, "and no lower than the clearance and the stack's padding: \(first - transcript.minY)")
    }
}
