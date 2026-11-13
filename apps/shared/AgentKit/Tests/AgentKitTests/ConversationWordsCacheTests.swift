import Foundation
import Testing

@testable import AgentKit

struct ConversationWordsCacheTests {
    @Test func theLedgerPreparesWordsBeforeHandingRowsToTheView() async throws {
        let ms = Int64.random(in: 1_000_000_000_000..<1_500_000_000_000)
        let origin = "ledger-\(UUID().uuidString)"
        let type = "ledger-\(UUID().uuidString)"
        let turn = AgentRow.Turn(
            prompt: "A turn", origin: origin, startedMs: 1, endedMs: ms + 1, durationMs: ms,
            outcome: .finished, backgroundRunning: 0, activity: nil)
        let rows = [
            AgentRow(id: "turn", ord: 0, rev: 1, kind: .turn(turn)),
            AgentRow(id: "tool", ord: 1, rev: 1, kind: .tool(.init(
                name: "Bash", summary: "A call", status: .done, startedMs: 10, endedMs: ms + 110,
                diff: [], filePath: nil))),
            AgentRow(id: "thinking", ord: 2, rev: 1, kind: .thinking(.init(startedMs: 20, endedMs: ms + 220))),
            AgentRow(id: "agent", ord: 3, rev: 1, kind: .subagent(.init(
                toolUseId: "agent", agentType: type, description: "A helper", background: false,
                status: .done, startedMs: 30, endedMs: nil, toolCount: 1, currentAction: "", lastMs: ms + 330))),
        ]
        let ledger = AgentRowLedger()
        let page = await ledger.replace(with: AgentRowPage(epoch: 1, rev: 1, moreBefore: false, rows: rows))
        #expect(page.rows == rows)
        #expect(ConversationWords.cachedTable() != nil,
                "the first row would have to fetch and decode its phrase table on the main actor")
        #expect(ConversationWords.itemCache.cached(.origin(origin)) as? Bool == false)
        #expect(ConversationWords.itemCache.cached(.agentType(type)) != nil)
        for value in [ms, ms + 100, ms + 200, ms + 300] {
            #expect(ConversationWords.itemCache.cached(.milliseconds(value)) != nil,
                    "the view would have to cross the bridge for \(value) on the main actor")
        }

        var changed = turn
        changed.durationMs = ms + 400
        let row = AgentRow(id: "turn", ord: 0, rev: 2, kind: .turn(changed))
        let follow = await ledger.apply(AgentRowChanges(epoch: 1, rev: 2, reset: false, changes: [.update(row)]))
        guard case .delta(let delta) = follow else { Issue.record("no delta"); return }
        #expect(delta.rows == [row])
        #expect(ConversationWords.itemCache.cached(.milliseconds(ms + 400)) != nil,
                "a follow must prepare changed words before it reaches the main actor too")
    }

    @Test func repeatedTimesCrossTheBridgeOnce() {
        let cache = ConversationWords.ItemCache(limit: 2)
        var calls = 0
        for _ in 0..<50 {
            let answer = cache.value(for: .milliseconds(8_000)) { request in
                calls += 1
                #expect(request.contains("8000"))
                return #"{"schema":1,"item":{"ms":"0:08"}}"#
            }
            #expect(answer as? String == "0:08")
        }
        #expect(calls == 1, "a first page must not serialize the same duration for every row")
    }

    @Test func fieldsStayDistinctAndFalseIsCached() {
        let cache = ConversationWords.ItemCache(limit: 4)
        var calls = 0
        let ask: (String) -> String? = { request in
            calls += 1
            return request.contains("agent_type")
                ? #"{"item":{"agent_type":"Typed"}}"#
                : #"{"item":{"origin":false}}"#
        }
        for _ in 0..<3 {
            #expect(cache.value(for: .origin("Typed"), ask: ask) as? Bool == false)
            #expect(cache.value(for: .agentType("Typed"), ask: ask) as? String == "Typed")
        }
        #expect(calls == 2)
    }

    @Test func failedAnswersAreRetried() {
        let failures: [String?] = [nil, "not JSON", #"{"item":{}}"#, #"{"item":{"ms":null}}"#, #"{"item":{"ms":false}}"#]
        for failure in failures {
            let cache = ConversationWords.ItemCache(limit: 2)
            var calls = 0
            let ask: (String) -> String? = { _ in
                calls += 1
                return calls == 1 ? failure : #"{"item":{"ms":"0:08"}}"#
            }
            #expect(cache.value(for: .milliseconds(8_000), ask: ask) == nil)
            #expect(cache.value(for: .milliseconds(8_000), ask: ask) as? String == "0:08")
            #expect(cache.value(for: .milliseconds(8_000), ask: ask) as? String == "0:08")
            #expect(calls == 2)
        }
    }

    @Test func aFullCacheEvictsOldAnswersAndStillCachesNewOnes() {
        let cache = ConversationWords.ItemCache(limit: 2)
        var calls = 0
        let ask: (String) -> String? = { _ in
            calls += 1
            return #"{"item":{"ms":"time from core"}}"#
        }
        for ms in [Int64(1), 2, 3, 3, 2] {
            #expect(cache.value(for: .milliseconds(ms), ask: ask) as? String == "time from core")
        }
        #expect(calls == 3)
        #expect(cache.cached(.milliseconds(1)) == nil)
        #expect(cache.cached(.milliseconds(2)) != nil)
        #expect(cache.cached(.milliseconds(3)) != nil)
        _ = cache.value(for: .milliseconds(1), ask: ask)
        #expect(calls == 4)
        #expect(cache.cached(.milliseconds(2)) == nil)
    }

    @Test func thePublicRowHelpersReachTheCache() {
        // Unusual inputs so another suite's normal fixtures cannot prime them.
        let ms: Int64 = 914_728_391_843
        let type = "cache-wiring-\(UUID().uuidString)"
        let origin = "cache-wiring-\(UUID().uuidString)"
        let time = AgentConversation.short(ms: ms)
        let name = AgentConversation.agentType(type)
        let turn = AgentRow.Turn(
            prompt: "", origin: origin, startedMs: nil, endedMs: nil, durationMs: nil,
            outcome: nil, backgroundRunning: 0, activity: nil)
        #expect(!AgentConversation.isNotice(turn))
        #expect(!time.isEmpty && name != type)
        #expect(ConversationWords.itemCache.cached(.milliseconds(ms)) as? String == time)
        #expect(ConversationWords.itemCache.cached(.agentType(type)) as? String == name)
        #expect(ConversationWords.itemCache.cached(.origin(origin)) as? Bool == false)
    }
}
