import Testing

@testable import AgentKit

// What an adapter test SAYS, pinned.
//
// Three apps held this copy privately and identically, so a one-word edit on
// one side would have shipped three spellings of one sentence with nothing
// failing. There is one implementation now, and changing what Test says has to
// come through here. Android mirrors these in
// `apps/android/.../model/AgentEventTest.kt`, word for word, because Kotlin
// cannot import this.

@Test func everyOutcomeSaysSomethingWrittenForAPerson() {
    // No outcome may render as nothing, and none may render its own case name
    // — a case name is an internal identifier and never belongs on screen.
    let all: [AdapterTestOutcome] = [
        .worked("Claude Code 2.0.1"),
        .failed(.formUnusable),
        .failed(.noAnswer(nil)),
        .failed(.noAnswer("error: no such runner")),
        .failed(.refused("could not find `npx` on this runner")),
    ]
    for outcome in all {
        #expect(!outcome.sentence.isEmpty)
        #expect(outcome.sentence.first?.isUppercase == true)
        for name in ["worked", "failed", "formUnusable", "noAnswer", "refused"] {
            #expect(!outcome.sentence.contains(name))
        }
    }
}

@Test func aRefusedHandshakeKeepsTheRunnersWordsOutOfTheSentence() {
    // The defect: `body["failure"]` drawn as the app's own red line on all
    // three platforms. What the daemon sends is a lowercase fragment about a
    // process, and it must still reach a person — it is the only clue about
    // which field is wrong — but it reaches one as output, in a `DetailBox`,
    // not in this app's voice.
    let said = "the adapter started and then went silent"
    let outcome = AdapterTestOutcome.failed(.refused(said))
    #expect(!outcome.sentence.contains(said))
    #expect(!outcome.sentence.contains(":"))
    #expect(outcome.sentence.hasSuffix("."))
    #expect(outcome.detail == said)
}

@Test func aRunnerThatNeverAnsweredKeepsTheCLIsLineAsOutput() {
    // The Mac's other half of the same rule: an exit whose output is not JSON
    // is the CLI talking, and that line used to BE the sentence.
    let outcome = AdapterTestOutcome.failed(.noAnswer("could not resolve host: box"))
    #expect(!outcome.sentence.contains("could not resolve host"))
    #expect(outcome.detail == "could not resolve host: box")
    // The phones have nothing to put there: a bridge call that throws throws a
    // state, not a message. The sentence is the same either way.
    #expect(AdapterTestOutcome.failed(.noAnswer(nil)).sentence == outcome.sentence)
}

@Test func anOutcomeWithNothingToShowReservesNoRoomForABox() {
    // An editor asks `detail` before it draws a box, so an empty one never
    // appears under a sentence looking like output that failed to arrive.
    #expect(AdapterTestOutcome.worked("Claude Code 2.0.1").detail == nil)
    #expect(AdapterTestOutcome.failed(.formUnusable).detail == nil)
    #expect(AdapterTestOutcome.failed(.noAnswer(nil)).detail == nil)
    // The daemon sends `failure` as "" on the success path, and a client that
    // lost the field decodes to the same thing. Neither is a transcript.
    #expect(AdapterTestOutcome.failed(.noAnswer("")).detail == nil)
    #expect(AdapterTestOutcome.failed(.refused("")).detail == nil)
}

@Test func aSuccessDoesNotClaimAProtocolItMayNotHaveSpoken() {
    // The defect the triplication was hiding. Only the Mac's `AdapterInfo`
    // carries a `backend`, so only the Mac can ask for `native` — and when it
    // does, the daemon runs codex's app-server or the Claude CLI's stream-json
    // handshake and no ACP is spoken at all. One sentence, true on two
    // platforms and false on the third.
    let native = AdapterTestOutcome.worked("codex app-server 0.44")
    #expect(!native.sentence.contains("ACP"))
    // The name is still the news, so it is still in the line.
    #expect(native.sentence.contains("codex app-server 0.44"))
}

@Test func onlyAWorkingAdapterIsDrawnAsGoodNews() {
    #expect(AdapterTestOutcome.worked("x").succeeded)
    #expect(!AdapterTestOutcome.failed(.formUnusable).succeeded)
    #expect(!AdapterTestOutcome.failed(.noAnswer(nil)).succeeded)
    #expect(!AdapterTestOutcome.failed(.refused("x")).succeeded)
}

@Test func theThreeWaysATestFailsDoNotSayTheSameThing() {
    // The trap `b357841` found in `GapReason`: two cases returning one
    // sentence, with an adapter's text spliced on the end of one of them
    // carrying the whole distinction. Take the splice out without separating
    // them and the app says one thing about two situations.
    let sentences = Set(
        [
            AdapterTestOutcome.failed(.formUnusable),
            .failed(.noAnswer(nil)),
            .failed(.refused("x")),
        ].map(\.sentence))
    #expect(sentences.count == 3)
}

@Test func theSentencesAreTheOnesAndroidWroteDown() {
    // The other half of the pin. Kotlin cannot import this module, so
    // `AgentEventTest.kt` holds these same three literals; without them on both
    // sides an edit here would sail through and the third platform would go on
    // saying the old thing. Change one and you have to change the other, which
    // is the point.
    #expect(
        AdapterTestOutcome.worked("codex app-server 0.44").sentence
            == "Starts and answers — codex app-server 0.44")
    #expect(
        AdapterTestOutcome.failed(.noAnswer(nil)).sentence == "That runner couldn’t be reached.")
    #expect(
        AdapterTestOutcome.failed(.refused("x")).sentence
            == "This adapter didn’t start and answer.")
    // The Mac's alone — it pipes the form to the CLI as JSON and can fail to
    // encode it, where a bridge call has no encoding step. Android has no such
    // case and no such sentence, deliberately.
    #expect(
        AdapterTestOutcome.failed(.formUnusable).sentence
            == "That adapter couldn’t be described.")
}
