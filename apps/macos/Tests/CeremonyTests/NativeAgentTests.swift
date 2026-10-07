import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Mac's native view for terminal-mode claude panes (ov-372), drawn in a
/// real offscreen window: the switch beside the terminal, the composer, and
/// the rows that say what happened to a send. Rows and their store are
/// AgentKit's and tested there (`AgentRowStoreTests`: follow diffs applied in
/// place, a reset or a failed call paging again); the real runner is
/// `NativeAgentRunnerTests`.
@MainActor
@Suite(.serialized)
struct NativeAgentTests {
    /// What the probed views were, by id, the last time the window drew.
    final class Seen {
        var ids: Set<String> = []
    }

    struct Probe<Content: View>: View {
        let seen: Seen
        let content: Content
        var body: some View {
            content
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    let _ = seen.ids = Set(probed.map(\.id))
                    Color.clear
                }
        }
    }

    /// A stand-in for the terminal surface that counts how often SwiftUI
    /// made it anew, which is what a respawned pane would be.
    final class SurfaceLife {
        var appeared = 0
        var disappeared = 0
        var births: Set<UUID> = []
        var focused: [Bool] = []
    }

    struct StandInSurface: View {
        let life: SurfaceLife
        let focused: Bool
        @State private var birth = UUID()

        var body: some View {
            let _ = life.focused.append(focused)
            Color.black
                .identified("terminal-surface")
                .onAppear {
                    life.appeared += 1
                    life.births.insert(birth)
                }
                .onDisappear { life.disappeared += 1 }
        }
    }

    /// A runner that records what the composer sent and answers as told.
    actor StandInSink: ComposeSink {
        var sent: [String] = []
        /// Each send's images, in step with `sent`.
        var images: [[ComposeImage]] = []
        var answer: Result<Bool, RunnerCore.Failure> = .success(false)

        func set(_ answer: Result<Bool, RunnerCore.Failure>) { self.answer = answer }

        func compose(terminal: String, text: String, images: [ComposeImage]) async throws -> Bool {
            sent.append(text)
            self.images.append(images)
            return try answer.get()
        }
    }

    static func terminal(id: String = "0199aaaa-0000-7000-8000-000000000001", program: String = "claude", mode: String = "terminal") throws -> Terminal {
        try JSONDecoder().decode(
            Terminal.self,
            from: Data(#"{"id":"\#(id)","short":"t1","title":"claude","preset":"\#(program)","program":"\#(program)","state":"running","epoch":1,"paneMode":"\#(mode)"}"#.utf8))
    }

    static func window(_ view: some View) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(x: -9000, y: -9000, width: 720, height: 560), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: view.frame(width: 720, height: 560))
        window.orderFrontRegardless()
        return window
    }

    static func settle(_ window: NSWindow, _ ms: Int = 120) async {
        for _ in 0..<max(1, ms / 20) {
            window.contentView?.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    /// A registry offering the native view, with `model` as the pane's.
    static func agents(offering: Bool = true, rows: Bool = true, model: NativePaneModel? = nil) -> NativeAgents {
        let agents = NativeAgents(defaults: UserDefaults(suiteName: "native-agent-tests-\(UUID().uuidString)")!)
        agents.pretend(enabled: offering, rowsServed: rows, core: RunnerCore())
        if let model { agents.adopt(model) }
        return agents
    }

    static func model(_ terminal: Terminal, sink: (any ComposeSink)? = nil) -> NativePaneModel {
        NativePaneModel.remember(false, for: terminal.id)
        return NativePaneModel(terminal: terminal.id, store: AgentRowStore(key: "test-\(UUID())", cache: nil), sink: sink)
    }

    @Test("Switching views keeps the draft and never makes the terminal anew")
    func theSwitchKeepsTheDraftAndNeverRespawns() async throws {
        let terminal = try Self.terminal()
        let model = Self.model(terminal)
        let agents = Self.agents(model: model)
        let life = SurfaceLife()
        let seen = Seen()
        let window = Self.window(Probe(seen: seen, content: NativeSwitch(terminal: terminal, target: "", isFocused: true, agents: agents) { focused in
            StandInSurface(life: life, focused: focused)
        }))
        defer {
            window.close()
            NativePaneModel.remember(false, for: terminal.id)
        }
        await Self.settle(window)
        #expect(seen.ids.contains("native-switch") && seen.ids.contains("terminal-surface"))
        #expect(life.focused.last == true, "the terminal holds the keyboard first (R-27)")

        model.draft = "half a thought"
        for _ in 0..<3 {
            model.showsNative = true
            await Self.settle(window)
            #expect(life.focused.last == false, "the native view takes the keyboard")
            model.showsNative = false
            await Self.settle(window)
            #expect(life.focused.last == true)
        }
        #expect(life.appeared == 1 && life.disappeared == 0 && life.births.count == 1, "\(life.appeared) appeared, \(life.disappeared) went")
        #expect(model.draft == "half a thought")
        // Remembered per pane.
        model.showsNative = true
        #expect(NativePaneModel.remembered(for: terminal.id))
    }

    @Test("Where the view isn't offered, the terminal shows with no switch")
    func withoutTheViewTheTerminalShows() async throws {
        let claude = try Self.terminal()
        let shell = try Self.terminal(id: "0199aaaa-0000-7000-8000-000000000002", program: "shell")
        let chat = try Self.terminal(id: "0199aaaa-0000-7000-8000-000000000003", mode: "agent")
        let cases: [(String, NativeAgents, Terminal, String)] = [
            ("the setting off", Self.agents(offering: false), claude, ""),
            ("a runner without agent_rows", Self.agents(rows: false), claude, ""),
            ("a shell pane", Self.agents(), shell, ""),
            ("a chat pane", Self.agents(), chat, ""),
            ("another runner", Self.agents(), claude, "--host box"),
        ]
        for (why, agents, terminal, target) in cases {
            let life = SurfaceLife()
            let seen = Seen()
            let window = Self.window(Probe(seen: seen, content: NativeSwitch(terminal: terminal, target: target, isFocused: true, agents: agents) { focused in
                StandInSurface(life: life, focused: focused)
            }))
            await Self.settle(window)
            #expect(seen.ids.contains("terminal-surface"), "\(why): no terminal")
            #expect(!seen.ids.contains("native-switch") && !seen.ids.contains("native-agent-view"), "\(why): offered")
            #expect(life.focused.last == true, "\(why)")
            window.close()
        }
    }

    @Test("Enter sends the box through compose, on one line, and checks it first")
    func theComposerSendsThroughCompose() async throws {
        let sink = StandInSink()
        let model = Self.model(try Self.terminal(), sink: sink)
        model.draft = "fix the build\nthen the docs"
        #expect(model.draft == "fix the build then the docs", "a line break becomes a space as it arrives")
        await model.send()
        #expect(await sink.sent == ["fix the build then the docs"])
        #expect(model.draft.isEmpty && model.issue == nil && model.queued.isEmpty)

        // What claude reads as a command, and what's too long, never leave.
        model.draft = "/cost"
        await model.send()
        model.draft = String(repeating: "a", count: NativePaneModel.longest + 1)
        await model.send()
        #expect(await sink.sent.count == 1)
        #expect(model.issue == .said(NativePaneModel.tooLong))
    }

    @Test("A busy agent takes the message into its queue, and the view shows it Queued")
    func aBusyAgentShowsQueued() async throws {
        let sink = StandInSink()
        await sink.set(.success(true))
        let terminal = try Self.terminal()
        let model = Self.model(terminal, sink: sink)
        let seen = Seen()
        let window = Self.window(Probe(seen: seen, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        defer { window.close() }
        await Self.settle(window)
        #expect(!seen.ids.contains("native-queued"))
        model.draft = "and then the docs"
        await model.send()
        await Self.settle(window)
        #expect(model.queued == ["and then the docs"] && model.draft.isEmpty)
        #expect(seen.ids.contains("native-queued"))

        // The transcript's own Queued row takes over from the echo.
        model.store.apply(try await model.store.ledger.page(Self.page([
            Self.row(0, "queued:1", ["Queued": ["text": "and then the docs", "state": "Waiting", "at_ms": 1]]),
        ])))
        model.settleQueued()
        #expect(model.queued.isEmpty)
        await Self.settle(window)
        #expect(seen.ids.contains("native-queued") && seen.ids.contains("native-row-queued:1"))
    }

    @Test("A dialog in the terminal shows a Handoff row with Show Terminal")
    func aDialogShowsHandoff() async throws {
        let sink = StandInSink()
        await sink.set(.failure(.refused("a question came up", word: "resource-conflict", what: "dialog")))
        let model = Self.model(try Self.terminal(), sink: sink)
        model.showsNative = true
        let seen = Seen()
        let window = Self.window(Probe(seen: seen, content: NativeAgentView(model: model, isFocused: true, showTerminal: { model.showsNative = false })))
        defer { window.close() }
        model.draft = "carry on"
        await model.send()
        await Self.settle(window)
        #expect(model.issue == .handoff && model.draft == "carry on", "the draft stays for after the dialog")
        #expect(seen.ids.contains("native-handoff") && seen.ids.contains("native-handoff-show-terminal"))

        // And the projector's own Handoff row, for a panel it saw.
        model.issue = nil
        model.store.apply(try await model.store.ledger.page(Self.page([
            Self.row(0, "handoff:1", ["Handoff": ["reason": "A panel is open", "at_ms": 1]]),
        ])))
        await Self.settle(window)
        #expect(seen.ids.contains("native-handoff") && seen.ids.contains("native-row-handoff:1"))

        // A draft in the terminal's box (R-28) is its own refusal.
        await sink.set(.failure(.refused("there's a draft", word: "resource-conflict", what: "draft")))
        await model.send()
        #expect(model.issue == .draftInTerminal)
    }

    /// A runner that answers a page, then fails every follow.
    struct FailingAfterAPage: AgentRowSource {
        let page: Data
        func page(before: UInt64?, limit: Int) async throws -> Data { page }
        func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data {
            throw RunnerCore.Failure.lost("gone")
        }
    }

    @Test("Rows held when the runner stops answering are marked stale, and the box waits")
    func staleRowsAreSaidAndTheBoxWaits() async throws {
        let sink = StandInSink()
        let model = Self.model(try Self.terminal(), sink: sink)
        let seen = Seen()
        let window = Self.window(Probe(seen: seen, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        defer {
            window.close()
            model.store.stop()
        }
        model.store.start(FailingAfterAPage(page: Self.page([Self.row(0, "prose:1", ["Prose": ["text": "Earlier.", "conclusion": false, "at_ms": 1]])])))
        // A loaded CI runner can take more than one settle pass to draw the
        // banner after the store goes stale, so wait for what's drawn, not
        // for one pass (CI run 37608760054).
        let deadline = ContinuousClock.now + .seconds(30)
        while !(model.store.isStale && seen.ids.contains("native-row-prose:1") && seen.ids.contains("native-stale")),
              ContinuousClock.now < deadline {
            await Self.settle(window)
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(seen.ids.contains("native-row-prose:1") && seen.ids.contains("native-stale"))
        model.draft = "carry on"
        #expect(!model.canSend)
    }

    @Test("A pane follows its runner only while its conversation shows")
    func onlyAShownPaneFollows() throws {
        let model = Self.model(try Self.terminal())
        model.setOnScreen(true, by: UUID())
        model.source = FailingAfterAPage(page: Self.page([]))
        model.followIfShown()
        #expect(!model.store.isFollowing, "a pane on its terminal holds a follow")
        model.showsNative = true
        #expect(model.store.isFollowing)
        model.showsNative = false
        #expect(!model.store.isFollowing)
    }

    @Test("A send that may have arrived never says it wasn't sent")
    func aTimedOutSendMayHaveBeenSent() {
        #expect(NativePaneModel.issue(for: RunnerCore.Failure.timedOut("late")) == .said(NativePaneModel.mayHaveBeenSent))
        #expect(NativePaneModel.issue(for: RunnerCore.Failure.lost("dropped")) == .said(NativePaneModel.mayHaveBeenSent))
        #expect(NativePaneModel.issue(for: RunnerCore.Failure.lost("never left", notSent: true)) != .said(NativePaneModel.mayHaveBeenSent))
    }

    @Test("Only a runner that serves rows and compose is offered the view")
    func theViewNeedsRowsAndCompose() {
        #expect(NativeAgents.serves(["agent_rows", "agent_compose"]))
        #expect(!NativeAgents.serves(["agent_rows"]), "rows from before compose: every send would fail")
        #expect(!NativeAgents.serves(["agent_compose"]))
    }

    @Test("A turn nobody typed is a notice, never the person's message")
    func aNotificationTurnIsANotice() async throws {
        let model = Self.model(try Self.terminal())
        let seen = Seen()
        let window = Self.window(Probe(seen: seen, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        defer { window.close() }
        model.store.apply(try await model.store.ledger.page(Self.page([
            Self.row(0, "turn:n1", ["Turn": ["prompt": "Agent \"Count the lines\" finished", "origin": "Notification", "started_ms": 1, "ended_ms": 2, "duration_ms": 1, "outcome": "Finished", "background_running": 0, "activity": NSNull()]]),
        ])))
        await Self.settle(window)
        #expect(seen.ids.contains("native-notice-turn"))
        #expect(NativeCopy.agentType("general-purpose") == "General purpose")
        #expect(NativeCopy.short(ms: 42_000) == "0:42" && NativeCopy.short(ms: 220_000) == "3:40")
    }

    nonisolated static func row(_ ord: Int, _ id: String, _ kind: [String: Any]) -> [String: Any] {
        ["id": id, "ord": ord, "rev": 1, "turn": NSNull(), "provisional": false, "kind": kind]
    }

    static func page(_ rows: [[String: Any]]) -> Data {
        (try? JSONSerialization.data(withJSONObject: ["epoch": 1, "rev": 1, "moreBefore": false, "rows": rows])) ?? Data()
    }
}
