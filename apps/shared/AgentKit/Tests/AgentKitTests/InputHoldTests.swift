import Testing

@testable import AgentKit

/// A runner's `terminal.write`, scripted: each call takes the next outcome, and
/// records the bytes it was asked to write. `gate` lets a test hold a call open
/// while more keys are typed.
@MainActor
final class FakeRunner {
    var outcomes: [WriteOutcome]
    private(set) var sent: [[UInt8]] = []
    var onSend: (() async -> Void)?

    init(_ outcomes: [WriteOutcome]) { self.outcomes = outcomes }

    func write(_ bytes: [UInt8]) async -> WriteOutcome {
        sent.append(bytes)
        await onSend?()
        return outcomes.isEmpty ? .written : outcomes.removeFirst()
    }
}

private func b(_ s: String) -> [UInt8] { Array(s.utf8) }

@MainActor
@Test func aTimedOutWriteIsNeverSentAgain() async {
    let hold = InputHold()
    let runner = FakeRunner([.maybeSent])
    await hold.type(b("rm x\r"), send: runner.write)
    #expect(hold.held.isEmpty && !hold.isHolding, "a timed-out key may have arrived")
    #expect(hold.maybeLost)
    #expect(hold.sentence == "Some typing may not have reached the runner.")
    await hold.retry(send: runner.write)
    await hold.type(b("ls"), send: runner.write)
    #expect(runner.sent == [b("rm x\r"), b("ls")], "nothing was typed twice: \(runner.sent)")
}

@MainActor
@Test func aWriteThatNeverLeftIsHeldWithItsReason() async {
    let hold = InputHold()
    let runner = FakeRunner([.neverSent(.disconnected)])
    await hold.type(b("ls\r"), send: runner.write)
    #expect(hold.held == b("ls\r") && hold.reason == .disconnected && !hold.maybeLost)
    #expect(hold.sentence?.hasPrefix("Far Cooler lost the connection") == true)
}

@MainActor
@Test func aNewKeyJoinsTheHoldAndDoesNotFlushIt() async {
    let hold = InputHold()
    let runner = FakeRunner([.neverSent(.disconnected)])
    await hold.type(b("ls"), send: runner.write)
    await hold.type(b("\r"), send: runner.write)
    #expect(runner.sent == [b("ls")], "the new key must not reach the runner: \(runner.sent)")
    #expect(hold.held == b("ls\r"))
}

@MainActor
@Test func tryAgainSendsHeldThenNewInOrderOnce() async {
    let hold = InputHold()
    let runner = FakeRunner([.neverSent(.disconnected), .written])
    await hold.type(b("ls"), send: runner.write)
    await hold.type(b("\r"), send: runner.write)
    await hold.retry(send: runner.write)
    #expect(runner.sent == [b("ls"), b("ls\r")], "\(runner.sent)")
    #expect(hold.held.isEmpty && hold.sentence == nil)
    await hold.retry(send: runner.write)
    #expect(runner.sent.count == 2, "a second Try Again has nothing to send")
}

@MainActor
@Test func aFailedTryAgainKeepsTheHoldUntilAnAnswerConfirms() async {
    let hold = InputHold()
    let runner = FakeRunner([.neverSent(.disconnected), .neverSent(.refused), .written])
    await hold.type(b("ab"), send: runner.write)
    await hold.retry(send: runner.write)
    #expect(hold.held == b("ab") && hold.reason == .refused, "cleared before the answer")
    await hold.retry(send: runner.write)
    #expect(hold.held.isEmpty)
}

@MainActor
@Test func aKeyTypedWhileAnotherIsInFlightCannotOvertakeIt() async {
    let hold = InputHold()
    let runner = FakeRunner([.neverSent(.disconnected)])
    // While "a" is in flight, "b" is typed. "a" then fails.
    runner.onSend = {
        runner.onSend = nil
        await hold.type(b("b"), send: runner.write)
    }
    await hold.type(b("a"), send: runner.write)
    #expect(runner.sent == [b("a")], "b was sent past a failed a: \(runner.sent)")
    #expect(hold.held == b("ab"), "\(hold.held)")
}

@MainActor
@Test func keysTypedBehindAWriteGoOutTogetherAfterItSucceeds() async {
    let hold = InputHold()
    let runner = FakeRunner([.written, .written])
    runner.onSend = {
        runner.onSend = nil
        await hold.type(b("b"), send: runner.write)
        await hold.type(b("c"), send: runner.write)
    }
    await hold.type(b("a"), send: runner.write)
    #expect(runner.sent == [b("a"), b("bc")], "\(runner.sent)")
}

@MainActor
@Test func discardDropsTheHold() async {
    let hold = InputHold()
    let runner = FakeRunner([.neverSent(.disconnected)])
    await hold.type(b("ls"), send: runner.write)
    hold.discard()
    #expect(hold.held.isEmpty && hold.sentence == nil)
    await hold.retry(send: runner.write)
    #expect(runner.sent == [b("ls")])
}

@MainActor
@Test func heldInputIsCappedKeepingTheEarliestAndSaysSo() async {
    let hold = InputHold()
    let runner = FakeRunner([.neverSent(.disconnected)])
    await hold.type(b("x"), send: runner.write)
    await hold.type([UInt8](repeating: 0x79, count: 10_000), send: runner.write)
    #expect(hold.held.count == InputHold.cap)
    #expect(hold.held.first == 0x78, "the earliest bytes are the ones kept")
    #expect(hold.truncated && hold.sentence?.hasSuffix("Only the first 4 KB is kept.") == true)
}

@MainActor
@Test func aClosedPaneDropsTheHoldAndAnythingInFlight() async {
    let hold = InputHold()
    let runner = FakeRunner([.neverSent(.disconnected), .written])
    await hold.type(b("ls"), send: runner.write)
    hold.paneClosed()
    #expect(hold.held.isEmpty && hold.sentence == nil)
    await hold.retry(send: runner.write)
    #expect(runner.sent == [b("ls")], "a closed pane is never written to")

    // A write that fails after the pane closed doesn't resurrect a hold.
    let late = InputHold()
    let slow = FakeRunner([.neverSent(.disconnected)])
    slow.onSend = { late.paneClosed() }
    await late.type(b("x"), send: slow.write)
    #expect(late.held.isEmpty && late.sentence == nil)
}

@Test func onlyAProvablyUnsentFailureIsResendable() {
    #expect(WriteOutcome.failed(word: nil, disconnected: true, notSent: true) == .neverSent(.disconnected))
    #expect(WriteOutcome.failed(word: nil, disconnected: true, notSent: false) == .maybeSent)
    #expect(WriteOutcome.failed(word: RunnerRefusal.timedOutWord, disconnected: false, notSent: false) == .maybeSent)
    #expect(WriteOutcome.failed(word: "not-found", disconnected: false, notSent: false) == .neverSent(.refused))
    #expect(WriteOutcome.failed(word: nil, disconnected: false, notSent: false) == .maybeSent)
}

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
    #expect(
        RunnerRefusal.word(inAnswerLine: ["code": "not-found", "timed_out": true]) == "not-found")
    #expect(RunnerRefusal.word(inAnswerLine: ["ok": false, "timed_out": false]) == nil)
}
