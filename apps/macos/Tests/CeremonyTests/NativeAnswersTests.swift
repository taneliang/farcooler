import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A held ask answered from its row in the Mac's native view (ov-370, R-33):
/// Allow and Deny for a permission, a question's options and Send Answer,
/// Approve Plan and Keep Planning for a plan, each sent to the runner's
/// `terminal.agent_answer` for this pane and the ask's held id. A hold that
/// ended, or a runner that takes no answers, leaves Show Terminal alone. The
/// runner's side is `held_dialogs` in the daemon's tests.
@MainActor
@Suite(.serialized)
struct NativeAnswersTests {
    /// A runner that records each answer and answers as told.
    actor StandInAnswers: AgentAnswerSink {
        var sent: [String] = []
        var answers: [[String: String]] = []
        var answer: Result<Void, RunnerCore.Failure> = .success(())

        func set(_ answer: Result<Void, RunnerCore.Failure>) { self.answer = answer }

        func answer(terminal: String, ask: String, option: String, answers: [String: String]) async throws {
            sent.append("\(ask) \(option)")
            self.answers.append(answers)
            try answer.get()
        }
    }

    final class Seen {
        var views: [String: CGRect] = [:]
    }

    struct Probe<Content: View>: View {
        let seen: Seen
        let content: Content
        var body: some View {
            content
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = seen.views = Dictionary(
                            probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                        Color.clear
                    }
                }
        }
    }

    nonisolated static let height: CGFloat = 560

    struct Drawn {
        let model: NativePaneModel
        let sink: StandInAnswers
        let seen: Seen
        let window: NSWindow

        var ids: Set<String> { Set(seen.views.keys) }

        func click(_ id: String) throws {
            let frame = try #require(seen.views[id], "\(id): \(seen.views.keys.sorted())")
            let at = NSPoint(x: frame.midX, y: NativeAnswersTests.height - frame.midY)
            for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                window.sendEvent(
                    NSEvent.mouseEvent(
                        with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                        windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
            }
        }

        /// Wait, up to a slow runner's patience, for the sink to have heard
        /// `count` answers.
        func heard(_ count: Int) async -> [String] {
            let deadline = ContinuousClock.now + .seconds(30)
            while await sink.sent.count < count, ContinuousClock.now < deadline {
                try? await Task.sleep(for: .milliseconds(20))
            }
            await NativeAgentTests.settle(window)
            return await sink.sent
        }
    }

    /// The native view of one pane holding `ask`'s row.
    static func draw(_ ask: [String: Any], answers: Bool = true) async throws -> Drawn {
        let sink = StandInAnswers()
        let model = NativeAgentTests.model(try NativeAgentTests.terminal())
        model.answers = answers ? sink : nil
        let rows = [
            NativeAgentTests.row(0, "turn:p1", ["Turn": [
                "prompt": "Make the button.", "origin": "Typed", "started_ms": 1, "ended_ms": NSNull(),
                "duration_ms": NSNull(), "outcome": NSNull(), "background_running": 0, "activity": "Waiting",
            ]]),
            NativeAgentTests.row(1, "ask:toolu_1", ["Ask": ask]),
        ]
        model.store.apply(try await model.store.ledger.page(NativeAgentTests.page(rows)))
        let seen = Seen()
        let window = NativeAgentTests.window(Probe(seen: seen, content: NativeAgentView(model: model, isFocused: false, showTerminal: {})))
        await NativeAgentTests.settle(window, 240)
        return Drawn(model: model, sink: sink, seen: seen, window: window)
    }

    static func permission(held: String? = "hook-ask-p") -> [String: Any] {
        ["kind": "Permission", "text": "Bash touch x", "tool": "Bash", "asked_ms": 1, "answered_ms": NSNull(), "answered": false,
         "held": held as Any? ?? NSNull()]
    }

    static func question(held: String? = "hook-ask-q") -> [String: Any] {
        ["kind": "Question", "text": "Which color?", "tool": "AskUserQuestion", "asked_ms": 1, "answered_ms": NSNull(),
         "answered": false, "held": held as Any? ?? NSNull(),
         "questions": [[
            "question": "Which color?", "header": "Color", "multi_select": false,
            "options": [["label": "Red", "description": "Warm"], ["label": "Blue", "description": "Calm"]],
         ]]]
    }

    static func plan(held: String? = "hook-ask-x") -> [String: Any] {
        ["kind": "PlanExit", "text": "# Plan 1. Make the button blue.", "tool": "ExitPlanMode", "asked_ms": 1,
         "answered_ms": NSNull(), "answered": false, "held": held as Any? ?? NSNull(),
         "plan": "# Plan\n\n1. Make the button blue.\n2. Ship it."]
    }

    @Test("A held permission's Allow and Deny answer it for this pane, once each click")
    func permissionButtons() async throws {
        let drawn = try await Self.draw(Self.permission())
        defer { drawn.window.close() }
        #expect(drawn.ids.isSuperset(of: ["native-ask-allow", "native-ask-deny", "native-ask-show-terminal"]), "\(drawn.ids.sorted())")
        #expect(!drawn.ids.contains("native-ask-approve") && !drawn.ids.contains("native-ask-send-answer"))
        try drawn.click("native-ask-allow")
        #expect(await drawn.heard(1) == ["hook-ask-p allow"])
        try drawn.click("native-ask-deny")
        #expect(await drawn.heard(2) == ["hook-ask-p allow", "hook-ask-p deny"])
        #expect(drawn.model.answerIssues.isEmpty && drawn.model.answering == nil)
    }

    @Test("A held question offers its options, and Send Answer sends the one picked")
    func questionOptions() async throws {
        let drawn = try await Self.draw(Self.question())
        defer { drawn.window.close() }
        #expect(drawn.ids.isSuperset(of: ["native-ask-option-0-0", "native-ask-option-0-1", "native-ask-other-0", "native-ask-send-answer"]), "\(drawn.ids.sorted())")
        try drawn.click("native-ask-send-answer")
        await NativeAgentTests.settle(drawn.window)
        #expect(await drawn.sink.sent.isEmpty, "nothing picked: Send Answer waits")
        try drawn.click("native-ask-option-0-1")
        await NativeAgentTests.settle(drawn.window)
        try drawn.click("native-ask-send-answer")
        #expect(await drawn.heard(1) == ["hook-ask-q answer"])
        #expect(await drawn.sink.answers == [["Which color?": "Blue"]])
    }

    @Test("A held plan shows the plan, and Approve Plan and Keep Planning answer it")
    func planButtons() async throws {
        let drawn = try await Self.draw(Self.plan())
        defer { drawn.window.close() }
        #expect(drawn.ids.isSuperset(of: ["native-ask-plan", "native-ask-approve", "native-ask-keep-planning"]), "\(drawn.ids.sorted())")
        try drawn.click("native-ask-approve")
        #expect(await drawn.heard(1) == ["hook-ask-x allow"])
        try drawn.click("native-ask-keep-planning")
        #expect(await drawn.heard(2) == ["hook-ask-x allow", "hook-ask-x deny"])
    }

    @Test("A hold that ended, or a runner that takes no answers, leaves Show Terminal alone")
    func noHoldNoButtons() async throws {
        for (ask, answers) in [(Self.question(held: nil), true), (Self.permission(held: nil), true), (Self.plan(), false)] {
            let drawn = try await Self.draw(ask, answers: answers)
            defer { drawn.window.close() }
            #expect(drawn.ids.contains("native-ask-show-terminal"), "\(ask["kind"]!)")
            for button in ["native-ask-allow", "native-ask-deny", "native-ask-send-answer", "native-ask-approve", "native-ask-keep-planning"] {
                #expect(!drawn.ids.contains(button), "\(ask["kind"]!): \(button)")
            }
            await drawn.model.answer(drawn.model.store.box("ask:toolu_1").flatMap {
                if case .ask(let a) = $0.row.kind { return a } else { return nil }
            }!, option: "allow")
            #expect(await drawn.sink.sent.isEmpty, "nothing to answer")
        }
    }

    @Test("Answered from another device: the row says which, and offers nothing")
    func answeredElsewhere() async throws {
        var ask = Self.permission(held: nil)
        ask["answered_by"] = "iPhone"
        let drawn = try await Self.draw(ask)
        defer { drawn.window.close() }
        guard case .ask(let drawnAsk)? = drawn.model.store.box("ask:toolu_1")?.row.kind else { Issue.record("no ask"); return }
        #expect(AgentConversation.askTitle(drawnAsk) == "Answered on iPhone")
        #expect(!drawn.ids.contains("native-ask-allow") && !drawn.ids.contains("native-ask-show-terminal"))
    }

    @Test("A refused answer says why on its row")
    func refusalIsSaid() async throws {
        let drawn = try await Self.draw(Self.permission())
        defer { drawn.window.close() }
        await drawn.sink.set(.failure(.refused("Someone already answered this.", word: "resource-conflict", what: "not_held")))
        try drawn.click("native-ask-allow")
        _ = await drawn.heard(1)
        #expect(drawn.model.answerIssues["hook-ask-p"] == AgentConversation.answerIssue(what: "not_held"))
        #expect(drawn.ids.contains("native-ask-issue"))
    }
}
