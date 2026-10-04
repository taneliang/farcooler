import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A chat pane whose agent died (ov-174): one line beside the composer
/// whatever the transcript holds, a Restart that starts it again in place,
/// and the runner's refusal of a prompt said rather than swallowed.
@MainActor
struct AgentStoppedTests {
    private static let said = #"{"Message":{"role":"Agent","text":"Half an answer","parent":null}}"#

    private func terminal(failure: String?) throws -> Terminal {
        let word = failure.map { #","agentFailure":"\#($0)""# } ?? ""
        let json = #"""
            {"id":"t1","short":"t1","title":"Terminal 1","preset":"claude","state":"running",
             "epoch":1,"paneMode":"agent"\#(word)}
            """#
        return try JSONDecoder().decode(Terminal.self, from: Data(json.utf8))
    }

    private func stream(_ runner: AgentSendFailureTests.Runner, events: [String]) async -> AgentStream {
        let stream = AgentStream(terminal: "t1")
        stream.runnerForTesting = { args in try await runner.run(args) }
        runner.events = events
        await stream.pump()
        return stream
    }

    /// An agent that died after it said something says it stopped, not that
    /// it couldn't start; before a word, the old sentence stands.
    @Test func aStoppedAgentIsSaidUnderItsConversation() async throws {
        let runner = AgentSendFailureTests.Runner()
        let talking = await stream(runner, events: [Self.said])
        #expect(!talking.transcript.rows.isEmpty, "the fixture has a conversation")
        #expect(talking.stoppedLine(for: try terminal(failure: "adapter-failed")) == "The agent stopped")
        #expect(talking.stoppedLine(for: try terminal(failure: nil)) == nil)
        #expect(
            talking.stoppedLine(for: try terminal(failure: "not-authenticated"))
                == "This agent needs you to sign in")

        let empty = await stream(AgentSendFailureTests.Runner(), events: [])
        #expect(empty.stoppedLine(for: try terminal(failure: "adapter-failed")) == "The agent could not start")
    }

    /// No adapter says so on its own; Restart is offered for the rest.
    @Test func restartIsNotOfferedWithNoAdapter() async throws {
        let stream = await stream(AgentSendFailureTests.Runner(), events: [Self.said])
        #expect(!stream.restartOffered(for: try terminal(failure: "no-adapter")))
        #expect(stream.stoppedLine(for: try terminal(failure: "no-adapter")) == "No chat adapter is set up for this agent")
        #expect(stream.restartOffered(for: try terminal(failure: "adapter-failed")))
        #expect(stream.restartOffered(for: try terminal(failure: "not-authenticated")))
        #expect(!stream.restartOffered(for: try terminal(failure: nil)))
    }

    /// Restart asks the runner for agent mode again, on this pane.
    @Test func restartAsksForAgentModeAgain() async {
        let runner = AgentSendFailureTests.Runner()
        let stream = await stream(runner, events: [Self.said])
        await stream.restart()
        #expect(runner.calls("set-pane-mode") == [["terminal", "set-pane-mode", "t1", "agent", "--json"]])
        #expect(stream.failure == nil)

        runner.refusals["set-pane-mode"] = "error: no\ncode: not-found"
        await stream.restart()
        #expect(stream.failure?.action == .restart)
        #expect(stream.failure?.sentence.hasPrefix("The agent wasn’t restarted. ") == true)
    }

    /// The daemon's refusal of a prompt to a stopped agent reaches the
    /// composer, in words, and the echo comes back out.
    @Test func aPromptToAStoppedAgentSaysSo() async {
        let runner = AgentSendFailureTests.Runner()
        runner.refusals["agent-prompt"] = "error: The agent stopped. Restart it, then try again.\ncode: agent-stopped"
        let stream = await stream(runner, events: [Self.said])
        #expect(!(await stream.send("are you there")))
        #expect(stream.failure?.sentence == "Your message wasn’t sent. The agent stopped. Restart it, then try again.")
        #expect(!stream.transcript.rows.contains { $0.kind == .message(role: .user, text: "are you there", parent: nil) })
    }
}
