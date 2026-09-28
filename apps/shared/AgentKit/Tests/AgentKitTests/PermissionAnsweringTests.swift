import Testing

@testable import AgentKit

/// A permission card stays up until the runner takes the answer, and says so
/// when it doesn't.
struct PermissionAnsweringTests {
    /// Taken: the card comes down.
    @Test func aSentAnswerTakesTheCardDown() {
        var answering = PermissionAnswering()
        let r1 = answering.begin("hook-ask-1")
        #expect(r1)
        let r2 = answering.finish("hook-ask-1", .sent)
        #expect(r2)
        #expect(answering.sending == nil)
        #expect(answering.sentence(for: "hook-ask-1") == nil)
    }

    /// Refused: the card stays up, because the ask may still be held on the
    /// runner, and it says why.
    @Test func aFailedAnswerKeepsTheCardAndSaysSo() {
        var answering = PermissionAnswering()
        _ = answering.begin("hook-ask-1")
        let down = answering.finish(
            "hook-ask-1", PermissionAnswering.outcome(refusedWith: nil))
        #expect(!down)
        #expect(
            answering.sentence(for: "hook-ask-1")
                == "Your answer may not have reached the runner. Try again.")
        #expect(answering.sentence(for: "hook-ask-2") == nil)
    }

    /// Answered somewhere else, or ended: the card comes down without a word.
    @Test func aConflictTakesTheCardDownSilently() {
        var answering = PermissionAnswering()
        _ = answering.begin("hook-ask-1")
        let outcome = PermissionAnswering.outcome(refusedWith: "resource-conflict")
        #expect(outcome == .answeredElsewhere)
        let r3 = answering.finish("hook-ask-1", outcome)
        #expect(r3)
        #expect(answering.sentence(for: "hook-ask-1") == nil)
    }

    /// A refusal with a word this build knows gets that word's sentence,
    /// after one of its own. Never the runner's own text.
    @Test func aKnownRefusalIsSaidInFarCoolersWords() {
        let outcome = PermissionAnswering.outcome(refusedWith: "invalid-argument")
        guard case let .failed(sentence) = outcome else {
            Issue.record("invalid-argument is a failure, got \(outcome)")
            return
        }
        #expect(sentence.hasPrefix("The runner didn’t take your answer. "))
        #expect(sentence.contains(RunnerRefusal.invalidArgument.sentence))
    }

    /// One answer at a time: a second tap while the first is out sends
    /// nothing.
    @Test func aSecondTapWhileAnAnswerIsOutIsRefused() {
        var answering = PermissionAnswering()
        let r4 = answering.begin("hook-ask-1")
        #expect(r4)
        let r5 = answering.begin("hook-ask-1")
        #expect(!r5)
    }

    /// Trying again clears the old sentence while the new answer is out.
    @Test func tryingAgainClearsTheSentence() {
        var answering = PermissionAnswering()
        _ = answering.begin("hook-ask-1")
        _ = answering.finish("hook-ask-1", .failed("x"))
        let r6 = answering.begin("hook-ask-1")
        #expect(r6)
        #expect(answering.sentence(for: "hook-ask-1") == nil)
    }
}
