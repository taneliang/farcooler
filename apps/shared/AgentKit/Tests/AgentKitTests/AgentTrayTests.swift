import Foundation
import Testing

@testable import AgentKit

// ov-453: the agents at work in a pane, pinned above its composer, and the
// one opened in the pane's place. The rows are the projector's
// (`tray_tests.rs`); the words and the choice of who's listed are here.

enum TrayRows {
    static func turn(_ ord: Int, _ id: String, open: Bool = true, tokens: Int? = 52_100) -> [String: Any] {
        var turn: [String: Any] = ["prompt": "Polish the chat view", "origin": "Typed", "started_ms": 1_000, "background_running": 0]
        if !open { turn["outcome"] = "Finished"; turn["ended_ms"] = 9_000 }
        turn["tokens"] = tokens
        return row(ord, id, turn: nil, ["Turn": turn])
    }

    static func sub(_ ord: Int, _ id: String, status: String = "Running", agent: String? = nil, type: String = "general-purpose", tokens: Int? = nil) -> [String: Any] {
        var sub: [String: Any] = [
            "tool_use_id": id, "agent_type": type, "description": "ov-\(ord) the work", "background": true, "status": status,
            "started_ms": 2_000 + ord, "tool_count": 3, "current_action": "Bash Run the tests", "last_ms": 3_000,
        ]
        sub["agent_id"] = agent
        sub["tokens"] = tokens
        return row(ord, "sub:\(id)", ["Subagent": sub])
    }

    static func tool(_ ord: Int, _ id: String, running: Bool) -> [String: Any] {
        row(ord, id, ["Tool": ["name": "Bash", "summary": "Check the build", "status": running ? "Running" : "Done", "started_ms": 4_000, "diff": [Any]()]])
    }

    static func thinking(_ ord: Int, _ id: String, open: Bool) -> [String: Any] {
        var thinking: [String: Any] = ["started_ms": 4_000]
        if !open { thinking["ended_ms"] = 4_500 }
        return row(ord, id, ["Thinking": thinking])
    }

    static func row(_ ord: Int, _ id: String, turn: String? = "turn:p1", _ kind: [String: Any]) -> [String: Any] {
        var row: [String: Any] = ["id": id, "ord": ord, "rev": ord + 1, "provisional": false, "kind": kind]
        row["turn"] = turn
        return row
    }

    static func page(_ rows: [[String: Any]]) -> Data {
        RowFixture.json(["epoch": 1, "rev": 99, "moreBefore": false, "rows": rows])
    }

    @MainActor
    static func store(_ rows: [[String: Any]]) async throws -> AgentRowStore {
        let store = AgentRowStore(key: "tray-\(UUID())", cache: nil)
        store.apply(try await store.ledger.page(page(rows)))
        return store
    }
}

@MainActor
@Test("Nothing running, no tray: an ended agent is history, not the tray's")
func theTrayIsEmptyWhileNoAgentRuns() async throws {
    let store = try await TrayRows.store([TrayRows.turn(0, "turn:p1"), TrayRows.sub(1, "a", status: "Completed")])
    #expect(AgentTray.entries(store).isEmpty)
}

@MainActor
@Test("Main first, then each running agent in launch order, with its type, work, clock and tokens")
func theTrayListsMainThenEveryRunningAgent() async throws {
    let store = try await TrayRows.store([
        TrayRows.turn(0, "turn:p1"),
        TrayRows.sub(1, "a", agent: "a1", tokens: 87_200),
        TrayRows.sub(2, "b", status: "Completed"),
        TrayRows.sub(3, "c", type: "Explore"),
        TrayRows.tool(4, "tool:t1", running: true),
    ])
    let entries = AgentTray.entries(store)
    #expect(entries.map(\.id) == [AgentTray.mainID, "sub:a", "sub:c"])
    #expect(entries.map(\.title) == ["main", "General purpose", "Explore"])
    let main = entries[0]
    #expect(main.isMain && main.running && main.action == "Bash Check the build", "main's running call")
    #expect(main.tokens == 52_100 && main.startedMs == 1_000)
    let a = entries[1]
    #expect(a.description == "ov-1 the work" && a.action == "Bash Run the tests" && a.startedMs == 2_001)
    #expect(a.agentId == "a1" && a.tokens == 87_200)
    #expect(entries[2].agentId == nil, "not named yet: listed, not openable")
    #expect(AgentTray.summary(entries) == "2 agents running")
}

@MainActor
@Test("Main says it thinks, works, or waits for the agents it left running")
func mainSaysWhatItDoes() async throws {
    let thinking = try await TrayRows.store([TrayRows.turn(0, "turn:p1"), TrayRows.sub(1, "a"), TrayRows.thinking(2, "think:1", open: true)])
    #expect(AgentTray.entries(thinking).first?.action == "Thinking")
    let working = try await TrayRows.store([TrayRows.turn(0, "turn:p1"), TrayRows.sub(1, "a"), TrayRows.tool(2, "tool:t1", running: false)])
    #expect(AgentTray.entries(working).first?.action == "Working")
    let waiting = try await TrayRows.store([TrayRows.turn(0, "turn:p1", open: false), TrayRows.sub(1, "a")])
    let main = try #require(AgentTray.entries(waiting).first)
    #expect(main.action == "Waiting for agents" && !main.running && main.endedMs == 9_000)
}

@MainActor
@Test("An agent launched after the tray was drawn joins it; one that ends leaves it")
func theTrayFollowsTheRows() async throws {
    let store = try await TrayRows.store([TrayRows.turn(0, "turn:p1"), TrayRows.sub(1, "a")])
    #expect(store.subagentIds == ["sub:a"])
    let follow = RowFixture.follow(rev: 120, [
        ["kind": "insert", "id": "sub:b", "rev": 120, "row": TrayRows.sub(2, "b")],
        ["kind": "update", "id": "sub:a", "rev": 120, "row": TrayRows.sub(1, "a", status: "Completed")],
    ])
    guard case .delta(let delta) = try await store.ledger.follow(follow) else { Issue.record("a reset"); return }
    store.apply(delta)
    #expect(store.subagentIds == ["sub:a", "sub:b"])
    #expect(AgentTray.entries(store).map(\.id) == [AgentTray.mainID, "sub:b"])
}

@Test("Tokens read as claude's panel counts them, in short")
func tokensReadShort() {
    #expect(AgentTray.tokens(950) == "950 tokens")
    #expect(AgentTray.tokens(87_200) == "87.2K tokens")
    #expect(AgentTray.tokens(1_234_567) == "1.2M tokens")
    #expect(AgentTray.tokens(1) == "1 token")
}

/// A source that records which agent's rows it was asked for.
final class DrillSource: AgentRowSource, @unchecked Sendable {
    let agent: String?
    let opens: Bool
    private let lock = NSLock()
    private var _asked: [String] = []
    var asked: [String] { lock.withLock { _asked } }

    init(agent: String? = nil, opens: Bool = true) {
        self.agent = agent
        self.opens = opens
    }

    func page(before: UInt64?, limit: Int) async throws -> Data {
        lock.withLock { _asked.append("page \(agent ?? "pane")") }
        return TrayRows.page([TrayRows.turn(0, "turn:\(agent ?? "pane")")])
    }

    func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data {
        try await Task.sleep(for: .milliseconds(50))
        return RowFixture.follow(epoch: 1, rev: afterRev, [])
    }

    func subagent(_ agentId: String) -> (any AgentRowSource)? {
        opens ? DrillSource(agent: agentId) : nil
    }
}

@MainActor
@Test("An agent opens to its own rows, and back closes it; main and an unnamed agent don't open")
func anAgentOpensToItsOwnRows() async throws {
    let drill = AgentDrill()
    let source = DrillSource()
    let main = AgentTray.Entry(id: AgentTray.mainID, isMain: true, title: "main", running: true)
    #expect(!drill.open(main, from: source, pane: "p"))
    let unnamed = AgentTray.Entry(id: "sub:c", isMain: false, title: "Explore", running: true)
    #expect(!drill.open(unnamed, from: source, pane: "p"))
    #expect(!drill.open(row: "sub:a", agentId: "a1", from: DrillSource(opens: false), pane: "p"), "a runner without subagent_rows")
    #expect(drill.opened == nil)

    #expect(drill.open(row: "sub:a", agentId: "a1", from: source, pane: "p"))
    let opened = try #require(drill.opened)
    #expect(opened.id == "sub:a" && opened.agentId == "a1")
    for _ in 0..<100 where opened.store.ids.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
    #expect(opened.store.ids == ["turn:a1"], "the agent's own rows, not the pane's")
    #expect(opened.store.isFollowing)

    drill.follow(false)
    #expect(!opened.store.isFollowing, "off screen, no held follow")
    drill.follow(true)
    #expect(opened.store.isFollowing)
    drill.close()
    #expect(drill.opened == nil && !opened.store.isFollowing)
}
