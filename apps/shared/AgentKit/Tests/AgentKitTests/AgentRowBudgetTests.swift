import Darwin
import Foundation
import Testing

@testable import AgentKit

// ov-371's two budgets, measured through the production loop:
//
// - an update costs the main thread 2 ms or less with 50,000 rows held;
// - a pane coming back draws its first frame within 100 ms.
//
// Times, not counts, because the budget is a time. Each is held at the 90th
// percentile of many samples, so one preemption on a loaded machine isn't a
// failure, and printed for the report. Both were seen failing with the work
// put back on the main thread (the commit names the mutations).

@MainActor
@Suite(.serialized)
struct AgentRowBudgetTests {
    static let held = 50_000
    static let updates = 200

    static func rows(_ count: Int, rev: UInt64) -> [AgentRow] {
        (0..<count).map { i in
            AgentRow(
                id: "tool:toolu_\(String(format: "%08d", i))_padding_past_inline", ord: UInt64(i), rev: rev, turn: "turn:\(i / 50)",
                kind: i % 3 == 0
                    ? .prose(.init(text: "A paragraph of narration number \(i).", conclusion: false, atMs: Int64(i)))
                    : .tool(.init(name: "Bash", summary: "cargo test -p \(i)", status: .done, startedMs: Int64(i), endedMs: Int64(i + 5), diff: [], filePath: nil)))
        }
    }

    /// What claude streaming a 25.6K-character reply looks like to a follow:
    /// the newest prose row growing, with a tool row arriving now and then.
    static func follows(from rev: UInt64, after ord: Int) -> [ScriptedRows.Answer] {
        let words = "the quick brown fox jumps over the lazy dog with `code` and **bold** text "
        var text = ""
        var answers: [ScriptedRows.Answer] = []
        var rev = rev
        var ord = ord
        let streaming = "prose:streaming"
        let streamingOrd = ord + 1
        for i in 0..<updates {
            rev += 1
            var changes: [[String: Any]] = []
            if i == 0 {
                ord += 1
                changes.append(["kind": "insert", "id": streaming, "rev": rev, "row": RowFixture.prose(ord: ord, rev: rev, text: "", id: streaming)])
            } else if i % 10 == 0 {
                ord += 1
                changes.append(["kind": "insert", "id": "tool:new\(i)", "rev": rev, "row": RowFixture.prose(ord: ord, rev: rev, text: "tool \(i)", id: "tool:new\(i)")])
            } else {
                while text.count < 25_600 * i / updates { text += words }
                changes.append(["kind": "update", "id": streaming, "rev": rev, "row": RowFixture.prose(ord: streamingOrd, rev: rev, text: text, id: streaming)])
            }
            answers.append(.data(RowFixture.follow(rev: rev, changes)))
        }
        return answers
    }

    /// The calling thread's CPU time so far. Called on the main actor, the
    /// main thread's: everything it did, whoever asked it to.
    static func threadCPU() -> Duration {
        var info = thread_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<thread_basic_info>.size / MemoryLayout<natural_t>.size)
        let port = mach_thread_self()
        defer { mach_port_deallocate(mach_task_self_, port) }
        let ok = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { thread_info(port, thread_flavor_t(THREAD_BASIC_INFO), $0, &count) }
        }
        guard ok == KERN_SUCCESS else { return .zero }
        let micros = Int64(info.user_time.seconds + info.system_time.seconds) * 1_000_000
            + Int64(info.user_time.microseconds + info.system_time.microseconds)
        return .microseconds(micros)
    }

    static func p90(_ times: [Duration]) -> Duration {
        let sorted = times.sorted()
        return sorted[min(sorted.count - 1, sorted.count * 9 / 10)]
    }

    static func ms(_ d: Duration) -> Double {
        Double(d.components.seconds) * 1000 + Double(d.components.attoseconds) / 1e15
    }

    @Test("An update costs the main thread 2 ms or less with 50,000 rows held")
    func anUpdateIsCheapAtFiftyThousandRows() async throws {
        let cache = AgentRowCache(directory: nil)
        let rows = Self.rows(Self.held, rev: 1)
        cache.keep(AgentRowSnapshot(epoch: 1, rev: 1, moreBefore: false, rows: rows), for: "budget")
        let store = AgentRowStore(key: "budget", cache: cache)
        #expect(store.ids.count == Self.held)

        let source = ScriptedRows(pages: [], follows: Self.follows(from: 1, after: Self.held - 1))
        let cpu = Self.threadCPU()
        store.start(source)
        defer { store.stop() }
        await waitFor("every update", within: .seconds(60)) { store.applied >= Self.updates - 1 }
        // Everything the main thread did while the updates went through,
        // this test's own polling included: the honest per-update share.
        let mainShare = (Self.threadCPU() - cpu) / store.applied

        let times = store.applyTimes
        let p90 = Self.p90(times)
        print(String(
            format: "ROWS-BUDGET update at %d rows: %d updates, apply p50 %.3f ms, p90 %.3f ms, max %.3f ms; main thread CPU %.3f ms per update",
            Self.held, times.count, Self.ms(times.sorted()[times.count / 2]), Self.ms(p90), Self.ms(times.max() ?? .zero),
            Self.ms(mainShare)))
        #expect(store.ids.count == Self.held + 1 + (Self.updates - 1) / 10)
        #expect(p90 <= .milliseconds(2), "p90 \(Self.ms(p90)) ms")
        // The main thread's whole CPU time is held only when asked
        // (FARCOOLER_PERF=1, this suite run alone): in the full suite other
        // suites' main-actor tests run on the same thread at the same time.
        if ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1" {
            #expect(mainShare <= .milliseconds(2), "main thread \(Self.ms(mainShare)) ms per update")
        }
    }

    @Test("A pane coming back draws its first frame within 100 ms, from memory and from disk")
    func aReturningPaneDrawsAtOnce() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("agent-rows-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let cache = AgentRowCache(directory: directory)
        cache.keep(AgentRowSnapshot(epoch: 3, rev: 9, moreBefore: true, rows: Self.rows(AgentRowCache.rowsKept, rev: 9)), for: "pane")
        cache.flush()
        // A runner that takes its time: the first frame mustn't wait for it.
        let slow = { ScriptedRows(pages: [.hang], follows: [.hang], delay: .milliseconds(400)) }

        var memory: [Duration] = []
        var disk: [Duration] = []
        for _ in 0..<10 {
            let began = ContinuousClock.now
            let store = AgentRowStore(key: "pane", cache: cache)
            store.start(slow())
            await waitFor("the first frame from memory") { !store.ids.isEmpty }
            memory.append(ContinuousClock.now - began)
            store.stop()

            cache.dropMemory()
            let cold = ContinuousClock.now
            let fresh = AgentRowStore(key: "pane", cache: cache)
            fresh.start(slow())
            await waitFor("the first frame from disk") { !fresh.ids.isEmpty }
            disk.append(ContinuousClock.now - cold)
            #expect(fresh.ids.count == AgentRowCache.rowsKept)
            fresh.stop()
            // Put memory back the way a pane leaving would.
            cache.keep(AgentRowSnapshot(epoch: 3, rev: 9, moreBefore: true, rows: Self.rows(AgentRowCache.rowsKept, rev: 9)), for: "pane")
        }
        print(String(
            format: "ROWS-BUDGET first frame: memory p90 %.2f ms, disk p90 %.2f ms (max %.2f)",
            Self.ms(Self.p90(memory)), Self.ms(Self.p90(disk)), Self.ms(disk.max() ?? .zero)))
        #expect(Self.p90(memory) <= .milliseconds(100))
        // Disk is held only when asked (FARCOOLER_PERF=1): a loaded CI Mac
        // reads a file slower than any budget a person feels (84 ms seen at
        // load 25 here).
        if ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1" {
            #expect(Self.p90(disk) <= .milliseconds(100))
        }
    }
}
