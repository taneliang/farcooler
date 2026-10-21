import Foundation
import Testing

@testable import AgentKit

// ov-629: the reader's row stays where it was while older rows load above.
// The rule for which row is held and how far to scroll is `PlaceHold`; that
// an older page waits for a scroll to rest is `AgentRowStore`'s.

private func toolRow(_ id: String, ord: Int) -> [String: Any] {
    [
        "id": id, "ord": ord, "rev": 5, "turn": "turn:1", "provisional": false,
        "kind": ["Tool": [
            "name": "Bash", "summary": "run \(ord)", "status": "Done", "started_ms": 1, "ended_ms": 2, "diff": [Any](),
            "input": "command: true", "result": "",
        ]],
    ]
}

private func page(_ rows: [[String: Any]], moreBefore: Bool = false) -> Data {
    RowFixture.json(["epoch": 1, "rev": 5, "moreBefore": moreBefore, "rows": rows])
}

@MainActor
@Suite(.serialized)
struct PlaceHoldTests {
    /// A hold that has seen row "a" at `y` with `oldest` the list's oldest row.
    private static func seen(_ y: CGFloat = 100, oldest: String = "r1") -> PlaceHold {
        let hold = PlaceHold()
        _ = hold.report("a", y: y, oldest: oldest, kept: { _ in true }, following: false, driving: false)
        return hold
    }

    /// Rows came in above (the oldest row is now "r0" and "r1" is still held): row "a" reports its new place.
    private static func arrive(_ hold: PlaceHold, at y: CGFloat) -> PlaceHold.Move? {
        hold.report("a", y: y, oldest: "r0", kept: { $0 == "r1" }, following: false, driving: false)
    }

    @Test("A page that starts with a tool group is held by its oldest row, not the group's id")
    func aToolGroupAtTheTopIsHeldByRow() async throws {
        let store = AgentRowStore(key: "hold-\(UUID())", cache: nil)
        let ledger = store.ledger
        #expect(store.take(try await ledger.page(page([
            toolRow("tool:t2", ord: 20), toolRow("tool:t3", ord: 21), RowFixture.prose(ord: 22, rev: 5, text: "after"),
        ], moreBefore: true))))
        let group = try #require(store.items.first?.id)
        // The group's id names its first row with a prefix: never a row's id.
        #expect(group == "tools:tool:t2" && !store.ids.contains(group))
        let oldest = try #require(store.ids.first)
        #expect(oldest == "tool:t2")

        let hold = PlaceHold()
        _ = hold.report("tools:tool:t2", y: 100, oldest: oldest, kept: { store.ids.contains($0) }, following: false, driving: false)
        _ = hold.report("prose:22", y: 300, oldest: oldest, kept: { store.ids.contains($0) }, following: false, driving: false)

        // Held by the group's id (what the view did first), nothing would be
        // held: the group's id is never in the list, so the row "was lost".
        let byItem = PlaceHold()
        let group0 = try #require(store.items.first?.id)
        _ = byItem.report(group0, y: 100, oldest: group0, kept: { store.ids.contains($0) }, following: false, driving: false)
        _ = byItem.report("prose:22", y: 300, oldest: group0, kept: { store.ids.contains($0) }, following: false, driving: false)

        // An older tool call joins the group above it: the group's id changes,
        // and the row that was oldest is still held.
        #expect(store.take(try await ledger.older(page([toolRow("tool:t1", ord: 19)]))))
        #expect(store.items.first?.id == "tools:tool:t1" && store.items.first?.id != group)
        #expect(store.ids.contains(oldest) && store.ids.first == "tool:t1")
        // The prose row moved down 80 pt: the list goes with it.
        let target = hold.report("prose:22", y: 380, oldest: store.ids.first, kept: { store.ids.contains($0) }, following: false, driving: false)
        #expect(target == .offset(80))
        #expect(hold.isHolding)
        // The same arrival, held by the item's id: kept(group) is false, so the place is lost.
        let lost = byItem.report("prose:22", y: 380, oldest: store.items.first?.id, kept: { store.ids.contains($0) }, following: false, driving: false)
        #expect(lost == nil && !byItem.isHolding, "holding by an item id is the bug this test stands against")
    }

    @Test("A row already on the screen is held, and a new row reporting first is not")
    func aNewRowDoesNotBecomeTheReference() {
        let hold = Self.seen()
        // An older row reports first: nothing to compare it with.
        #expect(hold.report("old", y: 50, oldest: "r0", kept: { $0 == "r1" }, following: false, driving: false) == nil)
        #expect(!hold.isHolding)
        #expect(Self.arrive(hold, at: 600) == .offset(500))
        #expect(hold.isHolding)
    }

    @Test("A reader's finger, or a list on its tail, lets the held row go")
    func driversAndTailsRelease() {
        for (following, driving) in [(false, true), (true, false)] {
            let hold = Self.seen()
            #expect(Self.arrive(hold, at: 600) == .offset(500))
            #expect(hold.isHolding)
            let target = hold.report("a", y: 700, oldest: "r0", kept: { _ in true }, following: following, driving: driving)
            #expect(target == nil && !hold.isHolding, "following: \(following), driving: \(driving)")
            // And with nothing held, it isn't taken up again until rows come in.
            #expect(hold.report("a", y: 710, oldest: "r0", kept: { _ in true }, following: false, driving: false) == nil)
            #expect(!hold.isHolding)
        }
    }

    @Test("A correction waits for the last one to land, and one waiting behind it goes once it has")
    func theAskedGate() {
        let hold = Self.seen()
        #expect(Self.arrive(hold, at: 600) == .offset(500))
        // The scroll hasn't landed (offset 0, asked 500): another place is not acted on.
        #expect(Self.arrive(hold, at: 650) == nil)
        // It lands; what came behind it is done from there.
        #expect(hold.landed(at: 500) == 550)
        #expect(hold.landed(at: 550) == nil)
        // Landed with nothing behind it: nothing more to do.
        let plain = Self.seen()
        #expect(Self.arrive(plain, at: 600) == .offset(500))
        #expect(plain.landed(at: 500) == nil)
        // And a place that is right asks for nothing.
        #expect(Self.arrive(plain, at: 100) == nil)
    }

    @Test("The row held that leaves the screen stays held, to be put right when it comes back")
    func goneRows() {
        let hold = Self.seen()
        #expect(Self.arrive(hold, at: 600) == .offset(500))
        hold.gone("other")
        #expect(hold.isHolding)
        hold.gone("a")
        #expect(hold.isHolding, "pushed far down, the row is let go by the list at once, and comes back")
        // It comes back where the first scroll brought it: 40 pt short.
        #expect(hold.landed(at: 500) == nil)
        #expect(Self.arrive(hold, at: 140) == .offset(540))
    }

    @Test("Each settle starts the wait again, and a release stops it")
    func settleRestarts() async throws {
        let hold = PlaceHold()
        var done = 0
        hold.settle(after: 0.4) { done += 1 }
        try await Task.sleep(for: .milliseconds(250))
        hold.settle(after: 0.4) { done += 1 }
        try await Task.sleep(for: .milliseconds(250))
        #expect(done == 0, "the first wait ran out although another had begun")
        try await Task.sleep(for: .milliseconds(500))
        #expect(done == 1)
        hold.settle(after: 0.2) { done += 1 }
        hold.release()
        try await Task.sleep(for: .milliseconds(500))
        #expect(done == 1, "a released hold still finished")
    }

    @Test("A row pushed more than a screen away is brought into view first, then put right from where it lands")
    func aFarRowIsBroughtIn() {
        let hold = Self.seen(300)
        hold.viewport = 800
        // The page was five screens tall: the list's estimates between are too rough to scroll by.
        #expect(Self.arrive(hold, at: 4_300) == .row("a"))
        // The list has been taken to the row, which comes back 120 pt above where it was: that, now, is a small step.
        hold.offset = 5_000
        #expect(Self.arrive(hold, at: 180) == .offset(4_880))
        // Brought in once only: a second far report goes by offset.
        let again = Self.seen(300)
        again.viewport = 800
        #expect(Self.arrive(again, at: 4_300) == .row("a"))
        #expect(Self.arrive(again, at: 4_900) == .offset(4_600))
    }

    @Test("A row outside the viewport isn't the one held, one inside is")
    func theHeldRowIsInView() {
        let hold = PlaceHold()
        hold.viewport = 800
        _ = hold.report("above", y: -700, oldest: "r1", kept: { _ in true }, following: false, driving: false)
        _ = hold.report("inside", y: 300, oldest: "r1", kept: { _ in true }, following: false, driving: false)
        #expect(hold.report("above", y: 150, oldest: "r0", kept: { $0 == "r1" }, following: false, driving: false) == nil)
        #expect(!hold.isHolding)
        #expect(hold.report("inside", y: 1_150, oldest: "r0", kept: { $0 == "r1" }, following: false, driving: false) == .row("inside"), "850 pt is past the viewport")
    }

    // MARK: The store waits for a scroll to rest

    private static func stored() async throws -> AgentRowStore {
        let store = AgentRowStore(key: "wait-\(UUID())", cache: nil)
        #expect(store.take(try await store.ledger.page(RowFixture.page(rev: 5, from: 100, rows: 3, moreBefore: true))))
        return store
    }

    @Test("An older page that comes while a scroll is in progress goes in when it rests")
    func anOlderPageWaitsForTheScroll() async throws {
        let store = try await Self.stored()
        let before = store.ids
        store.scrollInProgress = true
        let older = try await store.ledger.older(RowFixture.page(rev: 5, from: 97, rows: 3))
        #expect(older.prepends)
        #expect(store.take(older))
        #expect(store.ids == before, "rows went in under a moving finger")
        store.scrollInProgress = false
        #expect(store.ids == ["prose:97", "prose:98", "prose:99"] + before)
    }

    @Test("What follows a waiting page waits behind it, in order, and out of order is for the window")
    func changesWaitBehindIt() async throws {
        let store = try await Self.stored()
        store.scrollInProgress = true
        #expect(store.take(try await store.ledger.older(RowFixture.page(rev: 5, from: 97, rows: 3))))
        let insert: [String: Any] = ["kind": "insert", "id": "prose:200", "rev": 6, "row": RowFixture.prose(ord: 200, rev: 6, text: "streamed")]
        guard case .delta(let follow) = try await store.ledger.follow(RowFixture.follow(rev: 6, [insert])) else {
            Issue.record("the follow reset")
            return
        }
        #expect(!follow.prepends && store.take(follow))
        #expect(store.ids == ["prose:100", "prose:101", "prose:102"], "a change went in ahead of the page it follows")
        store.scrollInProgress = false
        #expect(store.ids == ["prose:97", "prose:98", "prose:99", "prose:100", "prose:101", "prose:102", "prose:200"])
        // A change that doesn't follow on from the one waiting is refused: the loop takes the window.
        store.scrollInProgress = true
        var gap = AgentRowDelta()
        gap.base = store.shownVersion + 5
        gap.version = store.shownVersion + 6
        gap.prepends = true
        #expect(!store.take(gap))
    }

    @Test("A whole window goes straight in, and the changes that waited are dropped as covered")
    func aWindowGoesStraightIn() async throws {
        let store = try await Self.stored()
        store.scrollInProgress = true
        #expect(store.take(try await store.ledger.older(RowFixture.page(rev: 5, from: 97, rows: 3))))
        let window = await store.ledger.window()
        #expect(store.take(window))
        #expect(store.ids.first == "prose:97")
        store.scrollInProgress = false
        #expect(store.ids == ["prose:97", "prose:98", "prose:99", "prose:100", "prose:101", "prose:102"], "the waiting page went in twice")
    }

    @Test("Nothing waits when no scroll is in progress")
    func nothingWaitsAtRest() async throws {
        let store = try await Self.stored()
        #expect(store.take(try await store.ledger.older(RowFixture.page(rev: 5, from: 97, rows: 3))))
        #expect(store.ids.first == "prose:97")
    }

    // MARK: A scroll that never reaches rest

    private static let held = ["prose:97", "prose:98", "prose:99", "prose:100", "prose:101", "prose:102"]

    @Test("A scroll that sends nothing for a while is taken as over, and what waited goes in")
    func aSilentScrollIsTakenAsOver() async throws {
        let store = try await Self.stored()
        store.scrollTimeout = .milliseconds(150)
        store.scrolling()
        #expect(store.take(try await store.ledger.older(RowFixture.page(rev: 5, from: 97, rows: 3))))
        #expect(store.ids == ["prose:100", "prose:101", "prose:102"], "the page went in under the scroll")
        try await Task.sleep(for: .milliseconds(600))
        #expect(store.ids == Self.held, "the page waited on a scroll that never ended")
        #expect(!store.scrollInProgress)
    }

    @Test("Each sign of life from the scroll starts the wait again")
    func signsOfLifeRestartTheWait() async throws {
        let store = try await Self.stored()
        store.scrollTimeout = .milliseconds(400)
        store.scrolling()
        #expect(store.take(try await store.ledger.older(RowFixture.page(rev: 5, from: 97, rows: 3))))
        for _ in 0..<3 {
            try await Task.sleep(for: .milliseconds(250))
            store.scrolling()
        }
        #expect(store.ids == ["prose:100", "prose:101", "prose:102"], "a scroll that was moving was taken as over")
        try await Task.sleep(for: .milliseconds(900))
        #expect(store.ids == Self.held)
    }

    @Test("An app sent to the background, or a gesture cancelled, ends the scroll at once")
    func anInterruptedScrollEnds() async throws {
        let store = try await Self.stored()
        store.scrolling()
        #expect(store.take(try await store.ledger.older(RowFixture.page(rev: 5, from: 97, rows: 3))))
        store.scrollEnded()
        #expect(store.ids == Self.held && !store.scrollInProgress)
        // The wait is cancelled with it: nothing is left to run out later.
        store.scrollTimeout = .milliseconds(10)
        try await Task.sleep(for: .milliseconds(100))
        #expect(store.ids == Self.held)
    }

    @Test("A whole window taken mid-scroll clears what waited, so the next change goes straight in")
    func aWindowClearsWhatWaited() async throws {
        let store = try await Self.stored()
        store.scrolling()
        #expect(store.take(try await store.ledger.older(RowFixture.page(rev: 5, from: 97, rows: 3))))
        #expect(store.take(await store.ledger.window()))
        #expect(store.ids == Self.held)
        let insert: [String: Any] = ["kind": "insert", "id": "prose:200", "rev": 6, "row": RowFixture.prose(ord: 200, rev: 6, text: "streamed")]
        guard case .delta(let follow) = try await store.ledger.follow(RowFixture.follow(rev: 6, [insert])) else {
            Issue.record("the follow reset")
            return
        }
        // Still scrolling: it follows the window, which is what the view shows, and is not queued behind a page the window had.
        #expect(store.take(follow))
        #expect(store.ids == Self.held + ["prose:200"], "the change waited behind a page the window already carried")
        store.scrollEnded()
        #expect(store.ids == Self.held + ["prose:200"])
    }
}
