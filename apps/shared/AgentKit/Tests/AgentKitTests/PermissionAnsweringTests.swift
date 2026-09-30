import Foundation
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

    // MARK: - A lock-screen tap on a closed hook ask (ov-57, T0 contract C5)

    private static let until = Date(timeIntervalSince1970: 1_790_551_063)
    private static let before = until.addingTimeInterval(-10)
    private static let after = until.addingTimeInterval(10)

    /// A refusal as the FFI's failure line carries it: the code, the daemon's
    /// `what`, and prose that never contains the `what` (`error.rs`).
    private func closed(
        _ what: String?, request: String = "hook-ask-1", word: String? = "resource-conflict",
        until: Date? = until, now: Date
    ) -> GlanceAnswer.Closing? {
        var line: [String: Any] = ["error": "Someone already answered this."]
        if let word { line["code"] = word }
        if let what { line["what"] = what }
        return GlanceAnswer.closing(
            request: request, word: RunnerRefusal.word(inAnswerLine: line),
            what: RunnerRefusal.what(inAnswerLine: line), until: until, now: now)
    }

    /// Two devices raced and the other won inside the hold: the ask is over,
    /// and saying so keeps the buttons off.
    ///
    /// Mutation: `closing` ignoring `until`. Red: "Too late here…" before it.
    @Test func aNotHeldBeforeUntilSaysAnsweredElsewhere() {
        let closing = closed("not_held", now: Self.before)
        #expect(closing?.outcome == .over)
        #expect(closing?.message == "Answered on another device.")
    }

    /// Past the hold, `not_held` is the hold running out, not a rival.
    ///
    /// Mutation: the `until` comparison flipped. Red: "Answered on another
    /// device." after it.
    @Test func aNotHeldAfterUntilSaysTooLate() {
        let closing = closed("not_held", now: Self.after)
        #expect(closing?.outcome == .over)
        #expect(closing?.message == "Too late here. Answer it in the terminal.")
        #expect(closed("not_held", now: Self.until)?.message == closing?.message)
    }

    /// The verdict never reached the hook, so the dialog is still at the
    /// keyboard, whenever the tap was. Fed the failure line exactly as the FFI
    /// writes it: the cause is the `what` key, and the prose never names it.
    ///
    /// Mutation: the cause read from the prose rather than `what`. Red:
    /// "Answered on another device." inside the hold.
    @Test func aNotDeliveredLineSaysTooLate() throws {
        let text = #"{"ok":false,"code":"resource-conflict","what":"not_delivered","error":"The answer didn't reach the agent. Try again."}"#
        let line = try #require(
            try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any])
        #expect(RunnerRefusal.what(inAnswerLine: line) == "not_delivered")
        for now in [Self.before, Self.after] {
            let closing = GlanceAnswer.closing(
                request: "hook-ask-1", word: RunnerRefusal.word(inAnswerLine: line),
                what: RunnerRefusal.what(inAnswerLine: line), until: Self.until, now: now)
            #expect(closing?.outcome == .over)
            #expect(closing?.message == "Too late here. Answer it in the terminal.")
        }
    }

    /// A line with no `what` (a runner older than it) reads as `not_held`,
    /// by the hold's end, as before.
    ///
    /// Mutation: an absent `what` read as `not_delivered`. Red: "Too late
    /// here" inside the hold.
    @Test func aLineWithNoWhatReadsAsNotHeld() {
        #expect(RunnerRefusal.what(inAnswerLine: ["code": "resource-conflict"]) == nil)
        #expect(closed(nil, now: Self.before)?.message == "Answered on another device.")
    }

    /// With no `until` to go by, the sentence claims neither cause.
    ///
    /// Mutation: a missing `until` read as before it. Red: "Answered on another
    /// device."
    @Test func aNotHeldWithNoHoldEndClaimsNeitherCause() {
        let closing = closed("not_held", until: nil, now: Self.before)
        #expect(closing?.outcome == .over)
        #expect(closing?.message == "Answered elsewhere, or it timed out.")
    }

    /// Only a hook ask refused as `resource-conflict` is closed. Every other
    /// refusal keeps today's reading, which nil hands back to the caller.
    ///
    /// Mutation: the `hook-ask-` check removed. Red: a chat pane's conflict is
    /// read as over.
    @Test func onlyAHookAskRefusedAsAConflictIsClosed() {
        #expect(closed("not_held", request: "perm-7", now: Self.before) == nil)
        #expect(closed("not_held", word: "not-found", now: Self.before) == nil)
        #expect(closed("not_held", word: nil, now: Self.before) == nil)
    }

    /// Past `until` the tap is refused on the phone, before any connection:
    /// nothing is sent, and the buttons stay off.
    ///
    /// Mutation: the local refusal at `until` exactly removed (`>` for `>=`).
    /// Red: nil at `until`.
    @Test func aTapPastTheHoldIsRefusedHere() {
        #expect(GlanceAnswer.refusedHere(until: Self.until, now: Self.before) == nil)
        #expect(GlanceAnswer.refusedHere(until: nil, now: Self.after) == nil)
        for now in [Self.until, Self.after] {
            let refused = GlanceAnswer.refusedHere(until: Self.until, now: now)
            #expect(refused?.outcome == .over)
            #expect(refused?.message == "Too late here. Answer it in the terminal.")
        }
    }

    /// A card tap on a hook ask skips the stream replay: the daemon refuses a
    /// stale hook-ask id atomically, so the replay only costs time inside the
    /// hold. Any other permission is still checked first.
    ///
    /// Mutation: `verifyFirst` true for a hook ask. Red: a replay is asked for.
    @Test func aHookAskTapSkipsTheReplayAndOthersDoNot() {
        #expect(GlanceTapRoute.route(request: "hook-ask-1", until: Self.until, now: Self.before)
            == .send(verifyFirst: false))
        #expect(GlanceTapRoute.route(request: "hook-ask-1", until: nil, now: Self.after)
            == .send(verifyFirst: false))
        #expect(GlanceTapRoute.route(request: "perm-7", until: nil, now: Self.after)
            == .send(verifyFirst: true))
    }

    /// Past the hold, the route is a refusal on the phone, before connecting.
    ///
    /// Mutation: the local refusal skipped. Red: `.send`.
    @Test func aTapPastTheHoldIsRoutedToARefusal() {
        #expect(GlanceTapRoute.route(request: "hook-ask-1", until: Self.until, now: Self.until)
            == .refuse(GlanceAnswer.Closing(outcome: .over, message: ClosedAskWording.tooLate)))
    }
}
