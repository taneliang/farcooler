import Foundation
import Testing

@testable import AgentKit

// ov-612: the conversation and the agent tray stay current across a stop and
// a start. The client core's call can't be cancelled, so a stopped loop's
// follow still answers; that answer must change nothing, and every start and
// every page puts the ledger's whole window on screen, so the view can't
// drift from it.

/// A runner whose follows wait until the test answers them, and which keeps
/// answering after the loop that asked is gone, as the client core does.
actor GatedRows: AgentRowSource, Awaitable {
    var watchers: [Watcher<GatedRows>] = []
    private(set) var pageCalls = 0
    private(set) var followCalls: [UInt64] = []
    private var pages: [Data]
    private var held: [CheckedContinuation<Data, Error>] = []

    init(pages: [Data]) {
        self.pages = pages
    }

    func page(before: UInt64?, limit: Int) async throws -> Data {
        pageCalls += 1
        changed()
        if !pages.isEmpty { return pages.removeFirst() }
        return try await withCheckedThrowingContinuation { held.append($0); changed() }
    }

    func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data {
        followCalls.append(afterRev)
        return try await withCheckedThrowingContinuation { held.append($0); changed() }
    }

    var waiting: Int { held.count }

    /// Answer the oldest call still waiting.
    func answer(_ data: Data) {
        held.removeFirst().resume(returning: data)
        changed()
    }

    /// End every call still waiting.
    func release() {
        for call in held { call.resume(throwing: CancellationError()) }
        held = []
        changed()
    }
}

private enum Resync {
    static func update(_ row: [String: Any], rev: UInt64) -> Data {
        RowFixture.follow(rev: rev, [["kind": "update", "id": row["id"] ?? "", "rev": rev, "row": row]])
    }

    static func insert(_ row: [String: Any], rev: UInt64) -> Data {
        RowFixture.follow(rev: rev, [["kind": "insert", "id": row["id"] ?? "", "rev": rev, "row": row]])
    }
}

@MainActor
@Test("A follow that answers after the stop changes neither the ledger nor the view", .timeLimit(.minutes(1)))
func aFollowAnsweredAfterStopChangesNothing() async throws {
    let source = GatedRows(pages: [RowFixture.page(rev: 1, rows: 2)])
    let store = AgentRowStore(key: "late-\(UUID())", cache: nil)
    store.start(source)
    // The loop shows a page before it asks for the next, so a held follow
    // means the page is on screen.
    await source.until { $0.waiting == 1 }
    #expect(store.phase == .live)

    let old = store.feed
    store.stop()
    await source.answer(Resync.insert(RowFixture.prose(ord: 2, rev: 2, text: "late"), rev: 2))
    // The stopped loop ends on the answer, once the ledger has refused it
    // (or, before the fix, taken it).
    await ended(old)
    #expect(await store.ledger.cursor?.rev == 1, "the stopped loop moved the cursor")
    #expect(await store.ledger.count == 2, "the stopped loop's row reached the ledger")
    #expect(store.ids == ["prose:0", "prose:1"])
    await source.release()
}

@MainActor
@Test("A re-page gives back a row the view is missing", .timeLimit(.minutes(1)))
func aRepageRestoresAMissingRow() async throws {
    // The second page's rows are the ones the ledger holds, unchanged.
    let source = GatedRows(pages: [RowFixture.page(rev: 1, rows: 2), RowFixture.page(rev: 3, rows: 3, rowRev: { _ in 1 })])
    let store = AgentRowStore(key: "drift-\(UUID())", cache: nil)
    store.retryDelay = .milliseconds(1)
    store.start(source)
    defer { store.stop() }
    // The loop shows a page before it asks for the next, so a held follow
    // means the page is on screen.
    await source.until { $0.waiting == 1 }
    #expect(store.phase == .live)

    // The ledger holds a row the view never heard of: drift, however it came.
    _ = try await store.ledger.follow(Resync.insert(RowFixture.prose(ord: 2, rev: 1, text: "row 2"), rev: 2))
    #expect(store.box("prose:2") == nil)

    // The runner can't diff: the loop pages, and the page holds that row.
    await source.answer(RowFixture.follow(rev: 3, reset: true, []))
    // The second page is on screen before the loop's next follow is asked.
    await source.until { $0.pageCalls == 2 && $0.waiting == 1 }
    #expect(store.ids == ["prose:0", "prose:1", "prose:2"])
    #expect(store.box("prose:2")?.row.ord == 2, "every id in order has a box")
    #expect(store.items.count == 3)
    await source.release()
}

@MainActor
@Test("A subagent completed during a stop shows as finished after the resume, not in the tray", .timeLimit(.minutes(1)))
func aSubagentCompletedDuringAStopLeavesTheTray() async throws {
    let running = TrayRows.sub(1, "a")
    let completed = TrayRows.sub(1, "a", status: "Completed")
    let source = GatedRows(pages: [TrayRows.page([TrayRows.turn(0, "turn:p1"), running])])
    let store = AgentRowStore(key: "tray-stop-\(UUID())", cache: nil)
    store.start(source)
    defer { store.stop() }
    // The loop shows a page before it asks for the next, so a held follow
    // means the page is on screen.
    await source.until { $0.waiting == 1 }
    #expect(store.phase == .live)
    #expect(AgentTray.entries(store).count == 2, "main and the running agent")

    // Away and back: the stopped loop's follow and the new one are both out.
    store.stop()
    store.start(source)
    await source.until { $0.waiting == 2 }
    // The agent ends while the pane is away. The old follow answers first
    // and its loop is gone; then the runner tells the new loop the same.
    await source.answer(Resync.update(completed, rev: 100))
    await source.answer(Resync.update(completed, rev: 100))
    await source.until { $0.followCalls.count >= 3 }

    guard case .subagent(let sub)? = store.box("sub:a")?.row.kind else {
        Issue.record("no box for the agent")
        return
    }
    #expect(sub.status == .done)
    #expect(AgentTray.entries(store).isEmpty, "a finished agent is listed as running")
    await source.release()
}

@MainActor
@Test("A start shows the ledger's whole window, dropping a row the ledger no longer holds", .timeLimit(.minutes(1)))
func aStartShowsTheWholeWindow() async throws {
    let source = GatedRows(pages: [RowFixture.page(rev: 1, rows: 3)])
    let store = AgentRowStore(key: "window-\(UUID())", cache: nil)
    store.start(source)
    // The loop shows a page before it asks for the next, so a held follow
    // means the page is on screen.
    await source.until { $0.waiting == 1 }
    #expect(store.phase == .live)
    store.stop()
    await source.release()

    // Drift both ways while stopped: a row gone from the ledger, and a row
    // the view never got.
    _ = try await store.ledger.follow(RowFixture.follow(rev: 2, [
        ["kind": "remove", "id": "prose:0", "rev": 2],
        ["kind": "insert", "id": "prose:3", "rev": 2, "row": RowFixture.prose(ord: 3, rev: 2, text: "row 3")],
    ]))
    #expect(store.ids == ["prose:0", "prose:1", "prose:2"])

    store.start(source)
    defer { store.stop() }
    // The window is shown before the loop asks anything.
    await source.until { $0.waiting == 1 }
    #expect(store.ids == ["prose:1", "prose:2", "prose:3"])
    #expect(store.box("prose:0") == nil)
    #expect(store.box("prose:3") != nil)
    await source.release()
}

@MainActor
@Test("A start with nothing missed walks no window: the main thread never passes over every row for it", .timeLimit(.minutes(1)))
func aStartWithNothingMissedWalksNoWindow() async throws {
    let source = GatedRows(pages: [RowFixture.page(rev: 1, rows: 3)])
    let store = AgentRowStore(key: "no-walk-\(UUID())", cache: nil)
    store.start(source)
    // The loop shows a page before it asks for the next, so a held follow
    // means the page is on screen.
    await source.until { $0.waiting == 1 }
    #expect(store.phase == .live)
    store.stop()
    store.start(source)
    defer { store.stop() }
    await source.until { $0.waiting == 2 }
    #expect(store.reconciled == 0, "a start with the view current walked the whole window")
    await source.release()
}

@MainActor
@Test("An older page applied before a window taken earlier keeps its rows")
func anOlderPageSurvivesAnEarlierWindow() async throws {
    let store = AgentRowStore(key: "older-window-\(UUID())", cache: nil)
    store.apply(try await store.ledger.page(RowFixture.page(rev: 5, from: 100, rows: 2, moreBefore: true)))
    // The loop takes the window; before its hop lands, an older page lands.
    let window = await store.ledger.window()
    let older = try await store.ledger.older(RowFixture.page(rev: 5, from: 98, rows: 2))
    #expect(store.take(older))
    #expect(store.take(window))
    #expect(store.ids == ["prose:98", "prose:99", "prose:100", "prose:101"])
    #expect(store.box("prose:98") != nil, "the older page's rows were taken back")
}

@MainActor
@Test("A follow applied after an older page takes the whole window, so the older rows keep their boxes")
func aFollowAfterAnOlderPageTakesTheWindow() async throws {
    let store = AgentRowStore(key: "older-follow-\(UUID())", cache: nil)
    store.apply(try await store.ledger.page(RowFixture.page(rev: 5, from: 100, rows: 2, moreBefore: true)))
    guard case .delta(let follow) = try await store.ledger.follow(Resync.insert(RowFixture.prose(ord: 102, rev: 6, text: "new"), rev: 6)) else {
        Issue.record("no delta")
        return
    }
    let older = try await store.ledger.older(RowFixture.page(rev: 6, from: 98, rows: 2))
    // The older page's hop lands first: it doesn't follow on from the view.
    #expect(!store.take(older))
    #expect(store.take(await store.ledger.window()))
    #expect(store.take(follow))
    #expect(store.ids == ["prose:98", "prose:99", "prose:100", "prose:101", "prose:102"])
    #expect(store.ids.allSatisfy { store.box($0) != nil })
}
