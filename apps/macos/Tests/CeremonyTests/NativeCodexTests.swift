import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A codex pane's conversation view on the Mac (ov-416): offered where the
/// runner says it projects and composes into codex (`codex_view`), its words
/// naming Codex, and no Stop or Send Now, which the runner presses in claude
/// alone.
@MainActor
@Suite(.serialized)
struct NativeCodexTests {
    static func codex(mode: String = "terminal") throws -> Terminal {
        try NativeAgentTests.terminal(id: "0199aaaa-0000-7000-8000-0000000000c0", program: "codex", mode: mode)
    }

    static func agents(offered: Set<String>) -> NativeAgents {
        let agents = NativeAgents(defaults: UserDefaults(suiteName: "native-codex-tests-\(UUID().uuidString)")!)
        agents.pretend(enabled: true, rowsServed: true, core: RunnerCore(), offered: offered)
        return agents
    }

    @Test("A codex pane is offered the view where the runner serves codex, and only in a terminal")
    func offeredWhereServed() throws {
        let served = Self.agents(offered: ["agent_rows", "agent_compose", "compose", "codex_view"])
        #expect(served.offers(try Self.codex(), target: ""))
        #expect(!served.offers(try Self.codex(mode: "agent"), target: ""), "a chat pane has its own view")
        let before = Self.agents(offered: ["agent_rows", "agent_compose", "compose"])
        #expect(!before.offers(try Self.codex(), target: ""), "a runner that refuses every codex send")
        #expect(before.offers(try NativeAgentTests.terminal(), target: ""), "claude as before")
    }

    @Test("The model names Codex in its words, and offers no Stop while codex works")
    func theModelNamesCodex() async throws {
        let terminal = try Self.codex()
        let agents = Self.agents(offered: ["agent_rows", "agent_compose", "compose", "codex_view", "terminal_interrupt"])
        let model = agents.model(for: terminal.id, program: terminal.program)
        defer { NativePaneModel.remember(false, for: terminal.id) }
        #expect(model.agent == "Codex")
        #expect(model.keys != nil, "the runner serves the keys")
        model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([NativeInterruptTests.turn(working: true)])))
        #expect(model.working)
        #expect(!model.offersStop && !model.offersSendNow, "but presses them in claude alone")
        let busy = NativePaneModel.issue(for: RunnerCore.Failure.refused("conflict", word: "resource-conflict", what: "busy"), agent: model.agent)
        #expect(busy == .said("Codex is working and can’t take a message from here right now."))
        let typing = NativePaneModel.issue(for: RunnerCore.Failure.refused("conflict", word: "resource-conflict", what: "typing"), agent: model.agent)
        #expect(typing == .said("Someone typed in the terminal in the last 3 seconds, so the message wasn’t sent. Try again once they stop."))
        for what in ["picker", "too_tall", "unconfirmed", "left_at_shell", "unsupported"] {
            guard case .said(let words) = NativePaneModel.issue(for: RunnerCore.Failure.refused("x", word: "resource-conflict", what: what), agent: "Codex") else {
                Issue.record("\(what) isn't said")
                continue
            }
            #expect(words.contains("Codex") && !words.contains("Claude"), "\(what): \(words)")
        }
        model.program = "claude"
        #expect(model.offersStop, "the same turn in claude")
    }
}
