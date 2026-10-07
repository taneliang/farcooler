import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Stop and Send Now in the Mac's native view (ov-368): offered only while
/// claude works and the runner serves them, wired to the runner's
/// `terminal.interrupt` and `terminal.send_now` for this pane, ⌘. as Stop in
/// the focused pane, and every refusal said in words or not at all. The real
/// runner's answer is `NativeAgentRunnerTests`.
@MainActor
@Suite(.serialized)
struct NativeInterruptTests {
    /// A runner that records which keys it was asked to press, and answers
    /// as told.
    actor StandInKeys: InterruptSink {
        var pressed: [String] = []
        var answer: Result<Void, RunnerCore.Failure> = .success(())

        func set(_ answer: Result<Void, RunnerCore.Failure>) { self.answer = answer }

        func interrupt(terminal: String) async throws {
            pressed.append("stop \(terminal)")
            try answer.get()
        }

        func sendNow(terminal: String) async throws {
            pressed.append("send now \(terminal)")
            try answer.get()
        }
    }

    /// The newest turn, open and working (`Busy`), or ended.
    static func turn(working: Bool) -> [String: Any] {
        NativeAgentTests.row(0, "turn:p1", ["Turn": [
            "prompt": "Tidy the parser.", "origin": "Typed", "started_ms": 1, "ended_ms": working ? NSNull() : 2 as Any,
            "duration_ms": working ? NSNull() : 1 as Any, "outcome": working ? NSNull() : "Finished" as Any,
            "background_running": 0, "activity": working ? "Busy" : "Idle",
        ]])
    }

    static func queued(_ text: String, state: String = "Waiting") -> [String: Any] {
        NativeAgentTests.row(1, "queued:1", ["Queued": ["text": text, "state": state, "at_ms": 1]])
    }

    static func model(keys: StandInKeys?) throws -> NativePaneModel {
        let model = NativeAgentTests.model(try NativeAgentTests.terminal())
        model.keys = keys
        return model
    }

    @Test("Stop and Send Now show only while Claude works and the runner serves them")
    func offeredOnlyWhileWorking() async throws {
        let keys = StandInKeys()
        let model = try Self.model(keys: keys)
        let seen = NativeAgentTests.Seen()
        let window = NativeAgentTests.window(NativeAgentTests.Probe(seen: seen, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        defer { window.close() }
        #expect(!model.offersStop, "no turn yet")

        model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([Self.turn(working: true), Self.queued("and the docs")])))
        await NativeAgentTests.settle(window)
        #expect(model.working && model.offersStop)
        #expect(seen.ids.contains("native-stop") && seen.ids.contains("native-send-now"))

        model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([Self.turn(working: false), Self.queued("and the docs", state: "Sent")])))
        await NativeAgentTests.settle(window)
        #expect(!model.working && !model.offersStop)
        #expect(!seen.ids.contains("native-stop") && !seen.ids.contains("native-send-now"))

        model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([Self.turn(working: true), Self.queued("and the docs")])))
        model.keys = nil
        await NativeAgentTests.settle(window)
        #expect(!model.offersStop, "a runner without terminal_interrupt")
        #expect(!seen.ids.contains("native-stop") && !seen.ids.contains("native-send-now"))
    }

    @Test("A runner serving terminal_interrupt gives each pane Stop and Send Now; one without, neither")
    func theRunnersWordGivesTheKeys() throws {
        let terminal = try NativeAgentTests.terminal()
        for (offered, served) in [(Set(["agent_rows", "agent_compose", "terminal_interrupt"]), true), (Set(["agent_rows", "agent_compose"]), false)] {
            let agents = NativeAgents(defaults: UserDefaults(suiteName: "native-interrupt-tests-\(UUID().uuidString)")!)
            agents.pretend(enabled: true, rowsServed: true, core: RunnerCore(), offered: offered)
            #expect((agents.model(for: terminal.id).keys != nil) == served, "\(offered)")
        }
    }

    @Test("Stop and Send Now ask the runner to press their key in this pane")
    func theButtonsPressThisPanesKeys() async throws {
        let keys = StandInKeys()
        let model = try Self.model(keys: keys)
        model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([Self.turn(working: true), Self.queued("and the docs")])))
        await model.stop()
        await model.sendNow()
        #expect(await keys.pressed == ["stop \(model.terminal)", "send now \(model.terminal)"])
        #expect(model.issue == nil && model.pressing == nil)

        // Between turns, nothing is asked.
        model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([Self.turn(working: false)])))
        await model.stop()
        #expect(await keys.pressed.count == 2)
    }

    @Test("⌘. is Stop in the focused pane, and nowhere else")
    func commandPeriodStops() async throws {
        for focused in [true, false] {
            let keys = StandInKeys()
            let model = try Self.model(keys: keys)
            model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([Self.turn(working: true)])))
            let window = NativeAgentTests.window(NativeAgentView(model: model, isFocused: focused, showTerminal: {}))
            defer { window.close() }
            await NativeAgentTests.settle(window)
            let event = try #require(NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0, windowNumber: window.windowNumber,
                context: nil, characters: ".", charactersIgnoringModifiers: ".", isARepeat: false, keyCode: 47))
            _ = window.performKeyEquivalent(with: event)
            let deadline = ContinuousClock.now + .seconds(30)
            while focused, await keys.pressed.isEmpty, ContinuousClock.now < deadline {
                try await Task.sleep(for: .milliseconds(20))
            }
            await NativeAgentTests.settle(window)
            #expect(await keys.pressed == (focused ? ["stop \(model.terminal)"] : []), "focused: \(focused)")
        }
    }

    @Test("A refusal is said in words, or not at all when there was nothing to do")
    func refusalsAreSaid() async throws {
        let refused = { (what: String) in RunnerCore.Failure.refused("no", word: "resource-conflict", what: what) }
        #expect(NativePaneModel.keyIssue(for: refused("idle"), .stop) == nil, "the turn ended on its own")
        #expect(NativePaneModel.keyIssue(for: refused("too_soon"), .stop) == nil, "a second click")
        #expect(NativePaneModel.keyIssue(for: refused("prompt"), .stop) == .handoff)
        #expect(NativePaneModel.keyIssue(for: refused("draft"), .sendNow) == .draftInTerminal)
        #expect(NativePaneModel.keyIssue(for: refused("unconfirmed"), .stop) == .said("Claude didn’t confirm it stopped. Check the terminal."))
        #expect(NativePaneModel.keyIssue(for: refused("unconfirmed"), .sendNow) == .said("Claude didn’t confirm it sent the queued messages. Check the terminal."))
        for word in ["not_an_agent", "unsupported", "unfamiliar", "unconfirmable", "not_running"] {
            if case .said(let words)? = NativePaneModel.keyIssue(for: refused(word), .stop) {
                #expect(words.hasSuffix("Use the terminal."), "\(word)")
            } else {
                Issue.record("\(word) says nothing")
            }
        }
        #expect(NativePaneModel.keyIssue(for: RunnerCore.Failure.timedOut("late"), .stop) == .said("The runner didn’t answer in time. Check the terminal."))

        // Through the model: the words land in the composer's issue line.
        let keys = StandInKeys()
        let model = try Self.model(keys: keys)
        model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([Self.turn(working: true)])))
        await keys.set(.failure(refused("typing")))
        await model.stop()
        #expect(model.issue == .said("Someone is typing in the terminal. Try again in a moment."))
        await keys.set(.failure(refused("prompt")))
        await model.sendNow()
        #expect(model.issue == .handoff)
    }
}
