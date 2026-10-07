import Foundation
import Testing

@testable import AgentKit

// ov-370: the rules a conversation view answers a held ask by, on the Mac and
// on a phone. The rows' buttons are held by each app's UI tests.

@Suite struct AgentAnswersTests {
    private let color = AgentRow.Ask.Question(
        question: "Which color?", header: "Color",
        options: [.init(label: "Red", description: "Warm"), .init(label: "Blue", description: "Calm")], multiSelect: false)
    private let sizes = AgentRow.Ask.Question(
        question: "Which sizes?", header: "Sizes",
        options: [.init(label: "S", description: ""), .init(label: "M", description: ""), .init(label: "L", description: "")],
        multiSelect: true)

    @Test func onlyAHeldUnansweredAskIsAnswerable() {
        let held = AgentRow.Ask(kind: "Question", text: "Which color?", tool: "AskUserQuestion", askedMs: 1, answered: false, held: "hook-ask-1")
        #expect(AgentConversation.answerable(held))
        var answered = held
        answered.answered = true
        #expect(!AgentConversation.answerable(answered), "answered in the record")
        var released = held
        released.held = nil
        #expect(!AgentConversation.answerable(released), "the hold ended: the terminal's")
    }

    @Test func answersNeedEveryQuestionAndKeepTheOfferedOrder() {
        let questions = [color, sizes]
        #expect(AgentConversation.answers(for: questions, picked: [0: ["Blue"]], typed: [:]) == nil, "sizes unanswered")
        #expect(AgentConversation.answers(for: questions, picked: [0: ["Blue"]], typed: [1: "   "]) == nil, "blank Other")
        let given = AgentConversation.answers(for: questions, picked: [0: ["Blue"], 1: ["L", "S"]], typed: [1: "XL"])
        #expect(given == ["Which color?": "Blue", "Which sizes?": "S, L, XL"], "offered order, then Other, joined as claude reads them")
        #expect(AgentConversation.answers(for: [color], picked: [:], typed: [0: "Green"]) == ["Which color?": "Green"])
        #expect(
            AgentConversation.answers(for: [color], picked: [0: ["Red"]], typed: [0: "Green"]) == ["Which color?": "Green"],
            "a single-choice question's Other replaces the pick")
        #expect(AgentConversation.answers(for: [], picked: [:], typed: [:]) == nil)
    }

    @Test func aPickReplacesOrToggles() {
        #expect(AgentConversation.pick("Blue", in: color, picked: ["Red"]) == ["Blue"])
        #expect(AgentConversation.pick("M", in: sizes, picked: ["S"]) == ["S", "M"])
        #expect(AgentConversation.pick("S", in: sizes, picked: ["S", "M"]) == ["M"])
    }

    @Test func theTitleNamesTheDeviceThatAnswered() {
        var ask = AgentRow.Ask(kind: "PlanExit", text: "# Plan", tool: "ExitPlanMode", askedMs: 1, answered: false, held: "hook-ask-2")
        #expect(AgentConversation.askTitle(ask) == "Claude has a plan for you to review")
        ask.held = nil
        ask.answeredBy = "iPhone"
        #expect(AgentConversation.askTitle(ask) == "Answered on iPhone", "before the record catches up")
        ask.answered = true
        #expect(AgentConversation.askTitle(ask) == "Answered on iPhone")
        ask.answeredBy = nil
        #expect(AgentConversation.askTitle(ask) == "Answered", "the keyboard")
    }

    @Test func refusalsAreWords() {
        #expect(AgentConversation.answerIssue(what: "not_held").contains("isn’t waiting here"))
        #expect(AgentConversation.answerIssue(what: "not_delivered") == "The answer didn’t reach Claude. Answer in the terminal.")
        #expect(AgentConversation.answerIssue(what: "answers") == "Answer every question first.")
        #expect(AgentConversation.answerIssue(what: nil, timedOut: true).contains("didn’t answer in time"))
        #expect(!AgentConversation.answerIssue(what: "x").contains("_"), "no runner words on screen")
    }
}
