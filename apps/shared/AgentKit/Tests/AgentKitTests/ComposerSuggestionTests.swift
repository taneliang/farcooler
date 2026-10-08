import Foundation
import Testing

@testable import AgentKit

// ov-409: claude's suggested next prompt as the composer's placeholder. It
// arrives on the newest turn's row (`suggestion`, from the runner's read of
// claude's box); the rules here decide when a composer offers it. Taking it
// is a draft, never a send.

@MainActor
@Suite struct ComposerSuggestionTests {
    private func turn(_ suggestion: String?, activity: String? = "Idle") -> AgentRow.Turn {
        AgentRow.Turn(
            prompt: "go", origin: "Typed", startedMs: 1, endedMs: 2, durationMs: 1, outcome: .finished, backgroundRunning: 0,
            activity: activity, suggestion: suggestion)
    }

    @Test func offeredOnlyForAnEmptyDraftOnLiveRowsOfAnAgentAtRest() {
        let words = "run the tests again"
        #expect(AgentConversation.suggestion(newestTurn: turn(words), draft: "", stale: false) == words)
        #expect(AgentConversation.suggestion(newestTurn: turn(words), draft: " \n", stale: false) == words, "blank is empty")
        #expect(AgentConversation.suggestion(newestTurn: turn(words), draft: "fix", stale: false) == nil, "typing wins")
        #expect(AgentConversation.suggestion(newestTurn: turn(words), draft: "", stale: true) == nil, "rows that may be old")
        #expect(AgentConversation.suggestion(newestTurn: turn(words, activity: "Busy"), draft: "", stale: false) == nil)
        #expect(AgentConversation.suggestion(newestTurn: turn(words, activity: "Waiting"), draft: "", stale: false) == nil)
        #expect(AgentConversation.suggestion(newestTurn: turn(nil), draft: "", stale: false) == nil)
        #expect(AgentConversation.suggestion(newestTurn: turn("  "), draft: "", stale: false) == nil)
        #expect(AgentConversation.suggestion(newestTurn: nil, draft: "", stale: false) == nil)
    }

    /// Through the real decoder and the store, from the fixture the Rust
    /// suite checks against what the runner writes.
    @Test func theStoreOffersTheNewestTurnsSuggestion() async throws {
        let fixture = try RowFixture.load()
        let store = AgentRowStore(key: "suggestion-\(UUID())", cache: nil)
        #expect(store.suggestion(draft: "") == nil)
        store.apply(try await store.ledger.page(fixture.page))
        // The fixture's turn is Busy: a hint, not a prediction.
        #expect(store.newestTurn?.suggestion == "run the tests again")
        #expect(store.suggestion(draft: "") == nil)

        // The same row once the agent rests: the bytes are the Rust golden's
        // shape, `"activity":"Idle"` in place of the Busy tag.
        let resting = RowFixture.json([
            "epoch": 5, "rev": 3, "moreBefore": false,
            "rows": [
                [
                    "id": "turn:p1", "ord": 0, "rev": 3, "turn": NSNull(), "provisional": false,
                    "kind": ["Turn": [
                        "prompt": "first", "origin": "Typed", "started_ms": 1, "ended_ms": NSNull(), "duration_ms": NSNull(),
                        "outcome": NSNull(), "background_running": 0, "activity": "Idle",
                        "suggestion": "wait for the background shell to finish",
                    ]],
                ]
            ],
        ])
        let other = AgentRowStore(key: "suggestion-\(UUID())", cache: nil)
        other.apply(try await other.ledger.page(resting))
        #expect(other.suggestion(draft: "") == "wait for the background shell to finish")
        #expect(other.suggestion(draft: "x") == nil)
    }
}
