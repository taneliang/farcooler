import Foundation
import Testing

@testable import AgentKit

// ov-452: the conversation view reads text first. A run of tool calls is one
// line, a scheduled task says it is one, a tool row opens to its input and
// result, and a task list is a checklist.

private func tool(_ name: String, _ summary: String, _ status: AgentRow.Status = .done) -> AgentRow.Tool {
    AgentRow.Tool(name: name, summary: summary, status: status, startedMs: nil, endedMs: nil, diff: [], filePath: nil)
}

@Suite("Conversation items")
struct ConversationItemsTests {
    /// Kinds by id prefix, as a store's rows have them.
    private func items(_ ids: [String]) -> [ConversationItem] {
        AgentConversation.items(ids) { id in
            if id.hasPrefix("tool") { return .tool }
            if id.hasPrefix("think") { return .thinking }
            return .other
        }
    }

    @Test("A run of calls and the thinking between them is one item; text, a subagent or a message ends it")
    func runsFold() {
        let ids = [
            "turn:1", "think:0", "tool:a", "think:1", "tool:b", "think:2", "tool:c", "think:3", "prose:1",
            "tool:d", "sub:1", "tool:e", "tool:f", "queued:1", "tool:g",
        ]
        #expect(items(ids) == [
            .row("turn:1"),
            .row("think:0"),
            .tools(id: "tools:tool:a", rows: ["tool:a", "think:1", "tool:b", "think:2", "tool:c"]),
            .row("think:3"),
            .row("prose:1"),
            .row("tool:d"),
            .row("sub:1"),
            .tools(id: "tools:tool:e", rows: ["tool:e", "tool:f"]),
            .row("queued:1"),
            .row("tool:g"),
        ])
    }

    @Test("A group's id is its first call's, so it keeps its identity as calls arrive")
    func stableIdentity() {
        #expect(items(["tool:a", "tool:b"]).map(\.id) == ["tools:tool:a"])
        #expect(items(["tool:a", "tool:b", "think:1", "tool:c"]).map(\.id) == ["tools:tool:a"])
    }

    @Test("The line says how many and what for, from the calls' own summaries")
    func titleAndPurpose() {
        let bash = [
            tool("Bash", "Map the conversation view code"), tool("Bash", "Find the row source and paste handling"),
            tool("Bash", "README check"),
        ]
        #expect(AgentConversation.groupTitle(bash) == "Ran 3 commands")
        #expect(AgentConversation.groupPurpose(bash) == "map the conversation view code, find the row source and paste handling, README check")
        let reads = [tool("Read", "/w/apps/NativeRows.swift"), tool("Read", "/w/apps/NativeRows.swift", .running)]
        #expect(AgentConversation.groupTitle(reads) == "Reading 2 files")
        #expect(AgentConversation.groupPurpose(reads) == "NativeRows.swift", "each said once")
        let mixed = [tool("Bash", "Get current time"), tool("CronCreate", "", .failed)]
        #expect(AgentConversation.groupTitle(mixed) == "Used 2 tools")
        #expect(AgentConversation.groupPurpose(mixed) == "get current time, CronCreate", "a call with no summary is named")
        #expect(AgentConversation.groupStatus(mixed) == .failed)
        #expect(AgentConversation.groupStatus(reads) == .running)
    }

    @Test("A long message folds; a short one doesn't")
    func longMessages() {
        #expect(!AgentConversation.isLong("Fix the build."))
        #expect(!AgentConversation.isLong((1...6).map { "line \($0)" }.joined(separator: "\n")))
        #expect(AgentConversation.isLong((1...7).map { "line \($0)" }.joined(separator: "\n")))
        #expect(AgentConversation.isLong(String(repeating: "word ", count: 120)), "one long paragraph wraps past six lines")
    }

    @Test("Where a message came from is said in words that say what happened")
    func origins() {
        let turn = { (origin: String) in
            AgentRow.Turn(prompt: "x", origin: origin, startedMs: nil, endedMs: nil, durationMs: nil, outcome: nil, backgroundRunning: 0, activity: nil)
        }
        #expect(AgentConversation.originNote(turn("Queued")) == "Queued during the last turn")
        #expect(AgentConversation.originNote(turn("Typed")) == nil)
        #expect(AgentConversation.isScheduled(turn("Scheduled")))
        #expect(!AgentConversation.isNotice(turn("Scheduled")), "a scheduled task is drawn as one, not as a notice")
        #expect(AgentConversation.queuedLabel("Sent") == "Sent mid-turn")
    }

    @Test("The polish fixture decodes a scheduled turn, a tool's input and result, and a checklist")
    func polishFixtureDecodes() throws {
        let data = try Data(contentsOf: RowFixture.root.appendingPathComponent("test/fixtures/agent-rows-polish.json"))
        let page = try AgentRowPage.decode(data)
        #expect(page.rows.map(\.id) == ["turn:p2", "tool:toolu_c", "tasks:turn:p2"])
        guard case .turn(let turn) = page.rows[0].kind else { Issue.record("not a turn"); return }
        #expect(AgentConversation.isScheduled(turn))
        guard case .tool(let tool) = page.rows[1].kind else { Issue.record("not a tool"); return }
        #expect(tool.input == "cron: 4 15 3 10 *\nrecurring: false")
        #expect(tool.result == "Scheduled f56f5668")
        #expect(tool.opens, "a call with no summary still opens")
        guard case .tasks(let tasks) = page.rows[2].kind else { Issue.record("not a task list"); return }
        #expect(tasks.items.map(\.subject) == ["Read the code", "Fix it", "Ship it"])
        #expect(tasks.items.map(\.status) == ["Completed", "InProgress", "Pending"])
        #expect(AgentConversation.taskProgress(tasks) == "1 of 3 done")
        #expect(tasks.items.map(AgentConversation.taskState) == ["Done", "In progress", "Not started"])
    }

    @Test("A store folds its rows into items as they arrive, and a change to a row doesn't regroup")
    @MainActor
    func storeItems() async throws {
        let store = AgentRowStore(key: "items-test", cache: nil)
        func toolRow(_ ord: Int) -> [String: Any] {
            [
                "id": "tool:\(ord)", "ord": ord, "rev": 1, "turn": "turn:1", "provisional": false,
                "kind": ["Tool": ["name": "Bash", "summary": "Step \(ord)", "status": "Done", "diff": [Any]()]],
            ]
        }
        let page = RowFixture.json(["epoch": 1, "rev": 1, "moreBefore": false, "rows": [
            RowFixture.prose(ord: 0, rev: 1, text: "Looking."), toolRow(1), toolRow(2), toolRow(3),
        ]])
        store.apply(try await store.ledger.page(page))
        #expect(store.items == [.row("prose:0"), .tools(id: "tools:tool:1", rows: ["tool:1", "tool:2", "tool:3"])])
        // A row's change (the call finishing) keeps the items as they were.
        var running = toolRow(3)
        running["rev"] = 2
        running["kind"] = ["Tool": ["name": "Bash", "summary": "Step 3", "status": "Failed", "diff": [Any]()]]
        let follow = RowFixture.follow(rev: 2, [["kind": "update", "id": "tool:3", "rev": 2, "row": running]])
        guard case .delta(let delta) = try await store.ledger.follow(follow) else { Issue.record("no delta"); return }
        store.apply(delta)
        #expect(store.items.count == 2)
    }
}
