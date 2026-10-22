import Foundation
import Observation
import Testing

@testable import AgentKit

// ov-691: the newest turn says whether claude is working (Stop, Send Now)
// and what the composer suggests. Finding it read every row after it, so a
// streamed word in the reply below it redrew whatever asked, the Mac's whole
// transcript and composer among them, 9 to 20 ms a word in a scroll.

/// A turn at `ord`, working or done.
private func turn(_ ord: Int, working: Bool) -> [String: Any] {
    [
        "id": "turn:\(ord)", "ord": ord, "rev": 1, "turn": NSNull(), "provisional": false,
        "kind": ["Turn": [
            "prompt": "Tidy the parser.", "origin": "Typed", "started_ms": 1, "background_running": 0,
            "outcome": working ? NSNull() : "Finished" as Any, "activity": working ? "Busy" : "Idle",
        ] as [String: Any]],
    ]
}

/// Whether reading `read` is told of a change while `change` runs.
@MainActor
private func notices(_ read: @escaping () -> Void, _ change: () async throws -> Void) async rethrows -> Bool {
    final class Told: @unchecked Sendable { var yes = false }
    let told = Told()
    withObservationTracking(read) { told.yes = true }
    try await change()
    return told.yes
}

@MainActor
@Test("The newest turn is read without reading the rows streaming after it")
func theNewestTurnIgnoresTheRowsAfterIt() async throws {
    let store = AgentRowStore(key: "newest-turn", cache: nil)
    let rows = [turn(1, working: false), turn(2, working: true), RowFixture.prose(ord: 3, rev: 1, text: "Reading")]
    store.apply(try await store.ledger.page(RowFixture.json(["epoch": 1, "rev": 1, "moreBefore": false, "rows": rows])))
    #expect(store.newestTurn?.activity == "Busy")
    #expect(store.box("turn:2")?.isTurn == true && store.box("prose:3")?.isTurn == false)

    // A word streams into the reply below the turn: nothing that reads the
    // newest turn hears of it.
    let word = try await notices({ _ = store.newestTurn }) {
        let follow = RowFixture.follow(rev: 2, [["id": "prose:3", "rev": 2, "kind": "update", "row": RowFixture.prose(ord: 3, rev: 2, text: "Reading the")]])
        guard case .delta(let delta) = try await store.ledger.follow(follow) else { Issue.record("reset"); return }
        store.apply(delta)
    }
    #expect(!word, "a streamed word was heard by whatever reads the newest turn")

    // The turn ending is heard, and read.
    let ended = try await notices({ _ = store.newestTurn }) {
        let follow = RowFixture.follow(rev: 3, [["id": "turn:2", "rev": 2, "kind": "update", "row": turn(2, working: false)]])
        guard case .delta(let delta) = try await store.ledger.follow(follow) else { Issue.record("reset"); return }
        store.apply(delta)
    }
    #expect(ended)
    #expect(store.newestTurn?.outcome == .finished)

    // So is a newer turn arriving.
    let newer = try await notices({ _ = store.newestTurn }) {
        let follow = RowFixture.follow(rev: 4, [["id": "turn:4", "rev": 1, "kind": "insert", "row": turn(4, working: true)]])
        guard case .delta(let delta) = try await store.ledger.follow(follow) else { Issue.record("reset"); return }
        store.apply(delta)
    }
    #expect(newer)
    #expect(store.newestTurn?.activity == "Busy")
}
