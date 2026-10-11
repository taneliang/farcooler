import Foundation
import Testing

@testable import AgentKit

// ov-474: claude's injected turns (a subagent's hand-back, a peer's message, a
// task notice, a reminder) arrive from the projector already read. Decoded
// from the page the client core writes from a redacted real transcript
// (`test/fixtures/agent-rows-harness.json`).

@Suite("Harness rows")
struct HarnessRowsTests {
    private func turns() throws -> [String: AgentRow.Turn] {
        let data = try Data(contentsOf: RowFixture.root.appendingPathComponent("test/fixtures/agent-rows-harness.json"))
        let page = try AgentRowPage.decode(data)
        var turns: [String: AgentRow.Turn] = [:]
        for row in page.rows {
            if case .turn(let turn) = row.kind { turns[row.id] = turn }
        }
        return turns
    }

    @Test("A hand-back is a report named by its agent's call, with its body clean and its agent's row to open")
    func report() throws {
        let turn = try #require(try turns()["turn:p2"])
        let harness = try #require(turn.harness)
        #expect(AgentConversation.isHarness(turn))
        #expect(harness.kind == "Report")
        #expect(AgentConversation.harnessTitle(harness) == "Report from review dead-layer removal")
        #expect(harness.body.hasPrefix("**Verdict: ready.**"))
        #expect(!harness.body.contains("[Subagent hand-back]") && !harness.body.contains("<agent-message"))
        #expect(!harness.body.split(separator: "\n", omittingEmptySubsequences: false).contains { $0.hasPrefix("  ") })
        let agent = try #require(AgentConversation.harnessAgent(harness))
        #expect(agent.row == "sub:toolu_017mbUTSqSamcXYDTCPmop2m")
        #expect(agent.agentId == "a1e51329cc3af58a4")
    }

    @Test("A peer no launch names is called by a short id; with no id at all, a subagent")
    func unnamed() throws {
        let harness = try #require(try turns()["turn:p5"]?.harness)
        #expect(harness.kind == "Message")
        #expect(AgentConversation.harnessTitle(harness) == "Message from agent a272e8")
        #expect(AgentConversation.harnessAgent(harness) == nil, "nothing to open")
        var bare = harness
        bare.agentId = nil
        #expect(AgentConversation.harnessTitle(bare) == "Message from a subagent")
    }

    @Test("A task notice is its one status line, and a reminder is a System note")
    func statusAndNote() throws {
        let all = try turns()
        let agent = try #require(all["turn:p3"]?.harness)
        #expect(AgentConversation.harnessTitle(agent) == "Agent finished: review dead-layer removal")
        let command = try #require(all["turn:p4"]?.harness)
        #expect(AgentConversation.harnessTitle(command) == "Background command finished: gh run watch 123456 --exit-status (exit 0)")
        let note = try #require(all["turn:p6"]?.harness)
        #expect(AgentConversation.harnessTitle(note) == "System note")
        #expect(note.body.hasPrefix("The user hasn’t heard") || note.body.hasPrefix("The user hasn't heard"))
    }

    @Test("What a person typed has no harness")
    func typed() throws {
        let turn = try #require(try turns()["turn:p1"])
        #expect(turn.harness == nil)
        #expect(!AgentConversation.isHarness(turn))
    }
}
