import Foundation
import Testing

@testable import AgentKit

struct ConversationWordsCacheTests {
    @Test func ledgerPreparesPhraseTableBeforeLayout() async {
        let row = AgentRow(id: "turn", ord: 0, rev: 1, kind: .turn(.init(
            prompt: "A turn", origin: "user", startedMs: 1, endedMs: 2, durationMs: 1,
            outcome: .finished, backgroundRunning: 0, activity: nil)))
        let ledger = AgentRowLedger()
        let page = await ledger.replace(with: AgentRowPage(epoch: 1, rev: 1, moreBefore: false, rows: [row]))
        #expect(page.rows == [row])
        #expect(ConversationWords.cachedTable() != nil,
                "the first row would fetch and decode its phrase table on the main actor")
    }
}
