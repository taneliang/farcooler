import Foundation
import Testing

@testable import AgentKit

// ov-371: a terminal's agent rows as an app holds them. Decoded from the one
// JSON shape the client core writes (`test/fixtures/agent-rows.json`, which
// the Rust suite checks against `rows_args`), applied by id off the main
// thread, and handed to the view one changed row at a time.

enum RowFixture {
    static var root: URL {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root
    }

    static func load() throws -> (page: Data, follow: Data) {
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/agent-rows.json"))
        let object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        return (
            try JSONSerialization.data(withJSONObject: try #require(object["page"])),
            try JSONSerialization.data(withJSONObject: try #require(object["follow"]))
        )
    }

    /// A page as the client core writes it, of `rows` prose rows numbered
    /// from `from`.
    static func page(epoch: UInt64 = 1, rev: UInt64, from: Int = 0, rows: Int, moreBefore: Bool = false, rowRev: ((Int) -> UInt64)? = nil) -> Data {
        let items = (from..<(from + rows)).map { prose(ord: $0, rev: rowRev?($0) ?? rev, text: "row \($0)") }
        return json(["epoch": epoch, "rev": rev, "moreBefore": moreBefore, "rows": items])
    }

    static func prose(ord: Int, rev: UInt64, text: String, id: String? = nil) -> [String: Any] {
        [
            "id": id ?? "prose:\(ord)", "ord": ord, "rev": rev, "turn": "turn:1", "provisional": false,
            "kind": ["Prose": ["text": text, "conclusion": false, "at_ms": 1_000]],
        ]
    }

    static func follow(epoch: UInt64 = 1, rev: UInt64, reset: Bool = false, _ changes: [[String: Any]]) -> Data {
        json(["epoch": epoch, "rev": rev, "reset": reset, "changes": changes])
    }

    static func json(_ object: Any) -> Data {
        (try? JSONSerialization.data(withJSONObject: object)) ?? Data()
    }
}

@Test("The shared fixture decodes every row kind to the values it holds")
func theSharedRowFixtureDecodes() throws {
    let fixture = try RowFixture.load()
    let page = try AgentRowPage.decode(fixture.page)
    #expect(page.epoch == 4 && page.rev == 9 && page.moreBefore)
    #expect(page.rows.map(\.id) == ["turn:p1", "prose:1", "think:1", "tool:toolu_1", "sub:toolu_2", "ask:1", "queued:1", "notice:1", "handoff:1", "gap:1"])

    guard case .turn(let turn) = page.rows[0].kind else { Issue.record("not a turn"); return }
    #expect(turn.prompt == "Fix the build\nand the tests")
    #expect(turn.origin == "Typed" && turn.activity == "Busy" && turn.backgroundRunning == 1)
    #expect(turn.startedMs == 1_000 && turn.durationMs == 60_000)
    #expect(turn.outcome == .failed("API error"))
    #expect(page.rows[0].turn == nil && page.rows[1].turn == "turn:p1")

    guard case .tool(let tool) = page.rows[3].kind else { Issue.record("not a tool"); return }
    #expect(tool.name == "Edit" && tool.summary == "src/main.rs" && tool.status == .done)
    #expect(tool.startedMs == 5_000 && tool.endedMs == 5_400 && tool.filePath == "/w/src/main.rs")
    #expect(tool.diff == [AgentRow.Hunk(oldStart: 3, oldLines: 1, newStart: 3, newLines: 1, lines: ["-a", "+b"])])

    guard case .subagent(let sub) = page.rows[4].kind else { Issue.record("not a subagent"); return }
    #expect(sub.agentType == "Explore" && sub.description == "Find the callers" && sub.background)
    #expect(sub.status == .ended("Killed") && sub.toolCount == 7 && sub.currentAction == "Grep fn main")
    #expect(sub.startedMs == 6_000 && sub.endedMs == nil && sub.lastMs == 9_000)

    #expect(page.rows[5].kind == .ask(.init(kind: "Permission", text: "Bash: rm -rf target", tool: "Bash", askedMs: 7_000, answered: false)))
    #expect(page.rows[6].kind == .queued(.init(text: "and then the docs", state: "Waiting", atMs: 8_000)))
    #expect(page.rows[7].kind == .notice(.init(kind: "Compacted", text: "Context compacted", atMs: nil)))
    #expect(page.rows[8].kind == .handoff(.init(reason: "A panel is open", atMs: 9_500)))
    #expect(page.rows[9].kind == .gap(.init(reason: "Unknown x-new", count: 2)))
    #expect(page.rows[2].kind == .thinking(.init(startedMs: 2_500, endedMs: 4_500)))
    #expect(page.rows[1].kind == .prose(.init(text: "Looking at **main.rs**.", conclusion: false, atMs: 2_000)))

    let follow = try AgentRowChanges.decode(fixture.follow)
    #expect(follow.epoch == 4 && follow.rev == 12 && !follow.reset)
    #expect(follow.changes.count == 3)
    guard case .remove(let gone, 12) = follow.changes[2] else { Issue.record("not a removal"); return }
    #expect(gone == "queued:1")
}

@MainActor
@Test("Follow diffs apply in place: one box per row, kept across updates")
func followDiffsApplyInPlace() async throws {
    let fixture = try RowFixture.load()
    let store = AgentRowStore(key: "in-place", cache: nil)
    store.apply(try await store.ledger.page(fixture.page))
    let toolBox = try #require(store.box("tool:toolu_1"))
    let proseBox = try #require(store.box("prose:1"))
    let ids = store.ids

    guard case .delta(let delta) = try await store.ledger.follow(fixture.follow) else {
        Issue.record("the follow reset")
        return
    }
    // Only what changed crosses to the main thread.
    #expect(delta.rows.map(\.id) == ["prose:2", "tool:toolu_1"])
    #expect(delta.removed == ["queued:1"])
    store.apply(delta)

    // The tool row changed in the box the view already holds.
    #expect(store.box("tool:toolu_1") === toolBox)
    guard case .tool(let tool) = toolBox.row.kind else { Issue.record("not a tool"); return }
    #expect(tool.status == .running && tool.endedMs == nil)
    // An untouched row is untouched.
    #expect(store.box("prose:1") === proseBox)
    // The inserted row is last, the removed one gone, the rest in place.
    #expect(store.ids == ids.filter { $0 != "queued:1" } + ["prose:2"])
    #expect(store.box("queued:1") == nil)
}

@Test("A follow from another projection, or one the runner can't diff, resets")
func aStaleFollowResets() async throws {
    let ledger = AgentRowLedger()
    _ = try await ledger.page(RowFixture.page(epoch: 1, rev: 5, rows: 3))
    #expect(try await ledger.follow(RowFixture.follow(epoch: 2, rev: 6, [])) == .reset)
    #expect(try await ledger.follow(RowFixture.follow(epoch: 1, rev: 6, reset: true, [])) == .reset)
    guard case .delta(let delta) = try await ledger.follow(RowFixture.follow(epoch: 1, rev: 6, [])) else {
        Issue.record("a same-epoch follow reset")
        return
    }
    #expect(delta.isEmpty)
}

@Test("A page of the same projection keeps the rows above it; another projection's replaces them")
func aPageKeepsOlderRowsOfTheSameProjection() async throws {
    let ledger = AgentRowLedger()
    _ = try await ledger.page(RowFixture.page(rev: 5, rows: 10))
    let again = try await ledger.page(RowFixture.page(rev: 6, from: 5, rows: 6, rowRev: { $0 == 10 ? 6 : 5 }))
    #expect(again.order == (0..<11).map { "prose:\($0)" })
    // Rows 5…9 are unchanged and don't cross again; 10 is new.
    #expect(again.rows.map(\.id) == ["prose:10"])
    let other = try await ledger.page(RowFixture.page(epoch: 2, rev: 1, from: 20, rows: 2))
    #expect(other.order == ["prose:20", "prose:21"])
    #expect(other.removed.count == 11)
}

@Test("An older page goes above what is held")
func anOlderPageGoesAbove() async throws {
    let ledger = AgentRowLedger()
    _ = try await ledger.page(RowFixture.page(rev: 5, from: 100, rows: 100, moreBefore: true))
    #expect(await ledger.oldestOrd == 100)
    let older = try await ledger.older(RowFixture.page(rev: 5, from: 0, rows: 100))
    #expect(older.order?.first == "prose:0" && older.order?.count == 200)
    #expect(!older.moreBefore)
}

/// A runner that answers from a script and records what it was asked.
actor ScriptedRows: AgentRowSource {
    enum Call: Equatable {
        case page(before: UInt64?)
        case follow(epoch: UInt64, afterRev: UInt64, waitMs: Int)
    }

    enum Answer {
        case data(Data)
        case fail
        case unavailable
        /// Hold the call until the test is over.
        case hang
    }

    private(set) var calls: [Call] = []
    private var pages: [Answer]
    private var follows: [Answer]
    private let delay: Duration

    init(pages: [Answer], follows: [Answer], delay: Duration = .zero) {
        self.pages = pages
        self.follows = follows
        self.delay = delay
    }

    func page(before: UInt64?, limit: Int) async throws -> Data {
        calls.append(.page(before: before))
        return try await answer(pages.isEmpty ? .hang : pages.removeFirst())
    }

    func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data {
        calls.append(.follow(epoch: epoch, afterRev: afterRev, waitMs: waitMs))
        return try await answer(follows.isEmpty ? .hang : follows.removeFirst())
    }

    private func answer(_ answer: Answer) async throws -> Data {
        if delay > .zero { try await Task.sleep(for: delay) }
        switch answer {
        case .data(let data): return data
        case .fail: throw URLError(.networkConnectionLost)
        case .unavailable: throw AgentRowsUnavailable()
        case .hang:
            try await Task.sleep(for: .seconds(3600))
            throw CancellationError()
        }
    }
}

@MainActor
func waitFor(_ what: String, within: Duration = .seconds(5), _ condition: () async -> Bool) async {
    let deadline = ContinuousClock.now + within
    while ContinuousClock.now < deadline {
        if await condition() { return }
        try? await Task.sleep(for: .milliseconds(5))
    }
    Issue.record("timed out waiting for \(what)")
}

@MainActor
@Test("The loop pages, follows, and pages again on a reset and after a failed call")
func theLoopRepagesOnResetAndFailure() async throws {
    let source = ScriptedRows(
        pages: [
            .data(RowFixture.page(rev: 2, rows: 2)),
            .data(RowFixture.page(epoch: 2, rev: 1, from: 10, rows: 1)),
            .data(RowFixture.page(epoch: 2, rev: 3, from: 10, rows: 2)),
        ],
        follows: [
            .data(RowFixture.follow(rev: 3, [["kind": "insert", "id": "prose:2", "rev": 3, "row": RowFixture.prose(ord: 2, rev: 3, text: "new")]])),
            .data(RowFixture.follow(rev: 3, reset: true, [])),
            .fail,
        ])
    let store = AgentRowStore(key: "loop", cache: nil)
    store.start(source)
    defer { store.stop() }
    await waitFor("the third page") { await source.calls.count >= 7 }
    let calls = await source.calls
    #expect(calls == [
        // Just paged, so the first follow waits for news.
        .page(before: nil),
        .follow(epoch: 1, afterRev: 2, waitMs: AgentRowStore.followWaitMs),
        .follow(epoch: 1, afterRev: 3, waitMs: AgentRowStore.followWaitMs),
        // The reset: page again.
        .page(before: nil),
        .follow(epoch: 2, afterRev: 1, waitMs: AgentRowStore.followWaitMs),
        // The failure: what was missed can't be named, so page again.
        .page(before: nil),
        .follow(epoch: 2, afterRev: 3, waitMs: AgentRowStore.followWaitMs),
    ])
    #expect(store.ids == ["prose:10", "prose:11"])
    #expect(store.phase == .live)
}

@MainActor
@Test("A runner without rows ends the loop and says so")
func aRunnerWithoutRowsIsUnavailable() async {
    let source = ScriptedRows(pages: [.unavailable], follows: [])
    let store = AgentRowStore(key: "unavailable", cache: nil)
    store.start(source)
    defer { store.stop() }
    await waitFor("unavailable") { store.phase == .unavailable }
    #expect(await source.calls == [.page(before: nil)])
}

@MainActor
@Test("A returning pane follows from the cached cursor instead of paging")
func aReturningPaneFollowsFromTheCache() async throws {
    let cache = AgentRowCache(directory: nil)
    cache.keep(AgentRowSnapshot(epoch: 7, rev: 40, moreBefore: true, rows: try AgentRowPage.decode(RowFixture.page(epoch: 7, rev: 40, rows: 3)).rows), for: "back")
    let source = ScriptedRows(pages: [], follows: [.data(RowFixture.follow(epoch: 7, rev: 41, []))])
    let store = AgentRowStore(key: "back", cache: cache)
    // Drawn before anything was asked.
    #expect(store.ids == ["prose:0", "prose:1", "prose:2"] && store.phase == .cached)
    store.start(source)
    defer { store.stop() }
    await waitFor("the catch-up follow") { await source.calls.count >= 2 }
    #expect(await source.calls.first == .follow(epoch: 7, afterRev: 40, waitMs: 0))
}

@MainActor
@Test("Starting again replaces the running follow: the first source is asked nothing more")
func startingAgainReplacesTheFollow() async throws {
    let quick = (0..<400).map { _ in ScriptedRows.Answer.data(RowFixture.follow(rev: 1, [])) }
    let first = ScriptedRows(pages: [.data(RowFixture.page(rev: 1, rows: 1))], follows: quick, delay: .milliseconds(5))
    let second = ScriptedRows(pages: [], follows: [.hang])
    let store = AgentRowStore(key: "again", cache: nil)
    store.start(first)
    defer { store.stop() }
    await waitFor("the first source following") { await first.calls.count >= 4 }
    store.start(second)
    await waitFor("the second source asked") { await second.calls.count >= 1 }
    let asked = await first.calls.count
    try await Task.sleep(for: .milliseconds(300))
    #expect(await first.calls.count <= asked + 1, "the old loop kept following")
    #expect(store.isStale == false)
}
