import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A chat pane's send, answer and setting change, when the runner doesn't
/// take them (ov-136).
///
/// Every one of these went out with `try?`. A refused ⌘↩ approval took its
/// card down anyway and left the agent blocked on an ask nobody could see; a
/// prompt that never left was cleared from the composer and drawn as sent; a
/// setting snapped to a value the agent never got. All of it was silent,
/// which is why it's held here: by looking, nothing is wrong.
///
/// Each test drives `AgentStream` the way `AgentSurface` and `AgentComposer`
/// call it, against a stub that stands in for the `farcooler` subprocess.
@MainActor
struct AgentSendFailureTests {
    /// Records each command line and answers it.
    @MainActor
    final class Runner {
        var calls: [[String]] = []
        /// What a call fails with, by subcommand; absent means it's taken.
        var refusals: [String: String] = [:]
        /// What `agent-subscribe` hands back, as event payloads.
        var events: [String] = []
        /// Holds every `agent-prompt` and `agent-answer` until opened, so a
        /// test can act while one is out.
        var held = false
        private var waiting: [CheckedContinuation<Void, Never>] = []

        var isHolding: Bool { !waiting.isEmpty }

        func open() {
            held = false
            let resume = waiting
            waiting = []
            for continuation in resume { continuation.resume() }
        }

        func run(_ args: [String]) async throws -> Data {
            let subcommand = args.count > 1 ? args[1] : ""
            if subcommand == "agent-subscribe" {
                let frames = events.enumerated().map { seq, payload in
                    ["seq": seq, "payloadJson": payload] as [String: Any]
                }
                events = []
                return try JSONSerialization.data(withJSONObject: ["events": frames, "epoch": 1])
            }
            calls.append(args)
            if held, subcommand == "agent-prompt" || subcommand == "agent-answer" {
                await withCheckedContinuation { waiting.append($0) }
            }
            if let refusal = refusals[subcommand] { throw AgentStream.StreamError.failed(refusal) }
            return Data()
        }

        func calls(_ subcommand: String) -> [[String]] {
            calls.filter { $0.count > 1 && $0[1] == subcommand }
        }
    }

    private static let permission = #"""
        {"Permission":{"id":"req-1","tool_call":"call-1","options":[\#
        {"id":"allow","name":"Allow","kind":"allow_once"},\#
        {"id":"deny","name":"Deny","kind":"reject_once"}]}}
        """#

    private static let session = #"""
        {"SessionStarted":{"session_id":"s","agent_mode":"default","available_modes":[],\#
        "model":"haiku","available_models":[],\#
        "config_options":[{"id":"model","name":"Model","description":"","category":"model",\#
        "kind":"select","current_value":"haiku",\#
        "options":[{"id":"haiku","name":"Haiku","description":""},\#
        {"id":"opus","name":"Opus","description":""}]}],\#
        "available_commands":[]}}
        """#

    /// A stream on `runner`, having read `events` once.
    private func stream(_ runner: Runner, events: [String] = []) async -> AgentStream {
        let stream = AgentStream(terminal: "t1")
        stream.runnerForTesting = { args in try await runner.run(args) }
        runner.events = events
        await stream.pump()
        return stream
    }

    /// Wait until the runner is holding a call.
    private func untilHeld(_ runner: Runner) async {
        while !runner.isHolding { await Task.yield() }
    }

    // MARK: - Answers

    /// A refused answer leaves the card up, saying why in this app's words,
    /// with the buttons back on. It used to come down on the click.
    @Test func aRefusedAnswerKeepsTheCardAndSaysWhy() async {
        let runner = Runner()
        runner.refusals["agent-answer"] = "error: scope denied for this device\ncode: scope-denied"
        let stream = await stream(runner, events: [Self.permission])
        #expect(stream.transcript.pendingPermission?.id == "req-1")

        await stream.answer("req-1", "allow")

        #expect(stream.transcript.pendingPermission?.id == "req-1", "the ask is still waiting")
        let said = stream.answering.sentence(for: "req-1")
        #expect(said?.hasPrefix("The runner didn’t take your answer. ") == true)
        #expect(said?.contains("scope") == false, "no raw CLI words")
        #expect(stream.answering.sending == nil)
        #expect(runner.calls("agent-answer") == [["terminal", "agent-answer", "t1", "req-1", "allow", "--json"]])
    }

    /// An answer refused because nothing holds that ask any more takes the
    /// card down without a word: it was answered elsewhere.
    @Test func anAskAnsweredElsewhereComesDownQuietly() async {
        let runner = Runner()
        runner.refusals["agent-answer"] = "error: not held\ncode: resource-conflict\nwhat: not_held"
        let stream = await stream(runner, events: [Self.permission])

        await stream.answer("req-1", "allow")

        #expect(stream.transcript.pendingPermission == nil)
        #expect(stream.answering.sentence(for: "req-1") == nil)
    }

    /// Try Again sends the option chosen the first time, and the card comes
    /// down once the runner takes it.
    @Test func aDroppedAnswerIsTriedAgainWithTheSameOption() async {
        let runner = Runner()
        runner.refusals["agent-answer"] = "error: connection reset"
        let stream = await stream(runner, events: [Self.permission])

        await stream.answer("req-1", "deny")
        #expect(
            stream.answering.sentence(for: "req-1")
                == "Your answer may not have reached the runner. Try again.")

        runner.refusals = [:]
        await stream.retryAnswer()

        #expect(runner.calls("agent-answer").map { $0[4] } == ["deny", "deny"])
        #expect(stream.transcript.pendingPermission == nil)
        #expect(stream.answering.sentence(for: "req-1") == nil)
    }

    /// A second click while an answer is out sends nothing.
    @Test func anAnswerGoesOnceWhileItIsOut() async {
        let runner = Runner()
        let stream = await stream(runner, events: [Self.permission])
        runner.held = true

        let first = Task { await stream.answer("req-1", "allow") }
        await untilHeld(runner)
        #expect(stream.answering.sending == "req-1")
        await stream.answer("req-1", "allow")
        await stream.retryAnswer()
        runner.open()
        await first.value

        #expect(runner.calls("agent-answer").count == 1)
        #expect(stream.transcript.pendingPermission == nil)
    }

    // MARK: - Sends

    /// A send that fails says false, so the composer keeps the words; says why
    /// beside the composer; and takes back the echo, so the words aren't also
    /// in the conversation looking sent.
    @Test func aFailedSendKeepsNothingLookingSent() async {
        let runner = Runner()
        runner.refusals["agent-prompt"] = "error: connection refused"
        let stream = await stream(runner)

        let sent = await stream.send("add tests")

        #expect(!sent)
        #expect(stream.failure == AgentActionFailure(.send, "Couldn’t reach this runner. Your message wasn’t sent."))
        #expect(!stream.transcript.rows.contains { $0.kind == .message(role: .user, text: "add tests", parent: nil) })
        #expect(runner.calls("agent-prompt") == [["terminal", "agent-prompt", "t1", "add tests", "--json"]])
    }

    /// A refusal the runner names is said in this app's sentence for it, after
    /// what didn't happen.
    @Test func aRefusedSendSaysWhyInItsOwnWords() async {
        let runner = Runner()
        runner.refusals["agent-prompt"] = "error: no\ncode: scope-denied"
        let stream = await stream(runner)

        #expect(!(await stream.send("add tests")))
        #expect(stream.failure?.sentence.hasPrefix("Your message wasn’t sent. This device can only look") == true)
    }

    /// A send that works says true and leaves the words in the conversation,
    /// and takes down the last send's failure.
    @Test func aSendThatWorksClearsTheFailure() async {
        let runner = Runner()
        runner.refusals["agent-prompt"] = "error: connection refused"
        let stream = await stream(runner)
        _ = await stream.send("add tests")
        runner.refusals = [:]

        #expect(await stream.send("add tests"))
        #expect(stream.failure == nil)
        let echoes = stream.transcript.rows.filter { $0.kind == .message(role: .user, text: "add tests", parent: nil) }
        #expect(echoes.count == 1, "drawn once, by the send that went")
    }

    /// Return pressed again, or Try Again, while a send is out sends nothing.
    @Test func aSendGoesOnceWhileItIsOut() async {
        let runner = Runner()
        let stream = await stream(runner)
        runner.held = true

        let first = Task { await stream.send("add tests") }
        await untilHeld(runner)
        #expect(stream.sending)
        #expect(!(await stream.send("add tests")))
        runner.open()

        #expect(await first.value)
        #expect(!stream.sending)
        #expect(runner.calls("agent-prompt").count == 1)
    }

    /// The composer empties only if it still holds what was sent: words typed
    /// while the send was out are the next message.
    @Test func theComposerKeepsWhatWasTypedDuringASend() {
        let picture = UUID()
        #expect(composerClears(afterSending: "hi", attachments: [picture], current: "hi\n", currentAttachments: [picture]))
        #expect(!composerClears(afterSending: "hi", attachments: [], current: "hi and", currentAttachments: []))
        #expect(!composerClears(afterSending: "hi", attachments: [picture], current: "hi", currentAttachments: []))
    }

    // MARK: - Settings and the queue

    /// A refused setting goes back to what the agent still has, says so, and
    /// Try Again sends it once more.
    @Test func aRefusedSettingSnapsBackAndCanBeTriedAgain() async {
        let runner = Runner()
        runner.refusals["agent-set-config"] = "error: refused\ncode: invalid-argument"
        let stream = await stream(runner, events: [Self.session])
        let model = { stream.transcript.configOptions.first { $0.id == "model" }?.currentValue }
        #expect(model() == "haiku")

        await stream.setConfig("model", "opus")

        #expect(model() == "haiku")
        #expect(stream.failure?.action == .config(id: "model", value: "opus"))
        #expect(stream.failure?.sentence.hasPrefix("The setting wasn’t changed.") == true)

        runner.refusals = [:]
        await stream.retry()
        await stream.retry()

        #expect(model() == "opus")
        #expect(stream.failure == nil)
        #expect(runner.calls("agent-set-config").count == 2, "the first try, and one Try Again")
    }

    /// A queue change that fails names the queued message, so its row carries
    /// the line.
    @Test func aFailedQueueChangeIsOnItsRow() async {
        let runner = Runner()
        runner.refusals["agent-steer-queued"] = "error: gone"
        let stream = await stream(runner)

        await stream.steerQueued("q-1")

        #expect(stream.failure?.action.queuedID == "q-1")
        #expect(stream.failure?.sentence == "Couldn’t reach this runner. The queued message wasn’t sent.")
    }

    // MARK: - A runner already gone

    /// On a runner already known to be gone, nothing runs and every refusal is
    /// on the thing refused, never in a field nothing draws.
    @Test func aRunnerAlreadyGoneRefusesWhereYouAsked() async {
        let runner = Runner()
        let stream = await stream(runner, events: [Self.permission])
        stream.refusal = { "ssh: connect to host mini port 22: Operation timed out" }

        #expect(!(await stream.send("add tests")))
        #expect(stream.failure?.sentence == "Couldn’t reach this runner. Your message wasn’t sent.")
        #expect(stream.transcript.rows.isEmpty, "no echo of something never sent")

        await stream.answer("req-1", "allow")
        #expect(stream.transcript.pendingPermission?.id == "req-1")
        #expect(stream.answering.sentence(for: "req-1") == "Couldn’t reach this runner. Your answer wasn’t sent.")

        #expect(runner.calls.isEmpty)
    }

    // MARK: - The chat itself

    /// A chat that can't be read says so in words once it has failed for a
    /// moment, and not at all for one failed poll among many.
    @Test func aChatThatCantReadSaysSoInWords() async {
        let stream = AgentStream(terminal: "t1")
        stream.runnerForTesting = { _ in throw AgentStream.StreamError.failed("error: broken pipe") }

        await stream.pump()
        #expect(stream.connectionError == nil)
        for _ in 1..<AgentStream.pollsBeforeSaying { await stream.pump() }
        #expect(stream.connectionError == "This chat isn’t updating. Trying again…")

        stream.runnerForTesting = { _ in try JSONSerialization.data(withJSONObject: ["events": [], "epoch": 0]) }
        await stream.pump()
        #expect(stream.connectionError == nil)
    }
}
