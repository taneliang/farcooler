import Testing

@testable import AgentKit

@Test func aTimedOutLineIsReadAsATimeoutAndSaysSoPlainly() {
    let line: [String: Any] = [
        "ticket": 4, "ok": false, "disconnected": false, "timed_out": true,
        "error": "The runner took too long to answer. Try again.",
    ]
    let word = RunnerRefusal.word(inAnswerLine: line)
    #expect(word == RunnerRefusal.timedOutWord)
    let trouble = RunnerRefusal.trouble(forWord: word, message: "raw", otherwise: "Generic.")
    #expect(trouble.sentence == RunnerRefusal.timedOutSentence)
    #expect(trouble.transcript == nil, "a timeout has no raw words to show beneath it")
    #expect(
        RunnerRefusal.trouble(forWord: word, message: "raw", after: "Couldn’t send it.").sentence
            == "Couldn’t send it. " + RunnerRefusal.timedOutSentence)
    // A refusal's own code still wins, and an ordinary line has no word.
    #expect(
        RunnerRefusal.word(inAnswerLine: ["code": "not-found", "timed_out": true]) == "not-found")
    #expect(RunnerRefusal.word(inAnswerLine: ["ok": false, "timed_out": false]) == nil)
}

@Test func aFailedWriteIsHeldAndSentAgainFirstAndInOrder() {
    let why = UnsentInput.why(word: RunnerRefusal.timedOutWord, disconnected: false)
    #expect(why == .timedOut)
    let held = UnsentInput(bytes: [0x6c, 0x73], why: why)
    #expect(UnsentInput.bytesToSend(after: held, adding: [0x0d]) == [0x6c, 0x73, 0x0d])
    #expect(UnsentInput.bytesToSend(after: nil, adding: [0x0d]) == [0x0d])
    #expect(held.holding([0x0d], why: .disconnected).bytes == [0x6c, 0x73, 0x0d])
    #expect(held.holding([0x0d], why: .disconnected).why == .disconnected)
}

@Test func theLineNamesTheReasonAndPromisesNothingLost() {
    #expect(UnsentInput.why(word: nil, disconnected: true) == .disconnected)
    #expect(UnsentInput.why(word: "not-found", disconnected: false) == .other)
    for why in [UnsentInput.Why.timedOut, .disconnected, .other] {
        let sentence = UnsentInput(bytes: [1], why: why).sentence
        #expect(sentence.hasSuffix("Your typing is waiting."), "\(sentence)")
        #expect(!sentence.contains("Error") && !sentence.contains("error"), "\(sentence)")
    }
    #expect(UnsentInput(bytes: [1], why: .timedOut).sentence.contains("took too long"))
    #expect(UnsentInput.retryTitle == "Try Again")
}
