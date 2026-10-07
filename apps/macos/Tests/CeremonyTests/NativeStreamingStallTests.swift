import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// ov-372's streaming check, by ov-382's method: the real `NativeAgentView`
/// in a real offscreen window, a session of 5,000 rows held, and a reply
/// growing to 25.6K characters in its newest prose row, one follow every
/// 52 ms, through the production store and loop. A thread pings the main
/// queue and records how late each ping ran (`StreamingReplyStallTests.
/// HangMonitor`); the budget is no stall over 50 ms while it streams.
///
/// Opt-in, as a timing harness on a shared machine can't be a gate: runs
/// only with FARCOOLER_PERF=1.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1"))
struct NativeStreamingStallTests {
    typealias HangMonitor = StreamingReplyStallTests.HangMonitor

    /// A runner whose session is `held` rows, then a reply streaming in.
    actor Streaming: AgentRowSource {
        let held: Int
        let chars: Int
        let every: Duration
        private var rev: UInt64 = 1
        private var text = ""
        private(set) var done = false

        init(held: Int, chars: Int, every: Duration) {
            self.held = held
            self.chars = chars
            self.every = every
        }

        func page(before: UInt64?, limit: Int) async throws -> Data {
            let from = max(0, held - limit)
            let rows = (from..<held).map { i -> [String: Any] in
                i % 4 == 0
                    ? NativeAgentTests.row(i, "turn:\(i)", ["Turn": ["prompt": "Prompt \(i)", "origin": "Typed", "started_ms": 1_000, "ended_ms": 9_000, "duration_ms": 8_000, "outcome": "Finished", "background_running": 0, "activity": NSNull()]])
                    : NativeAgentTests.row(i, "tool:\(i)", ["Tool": ["name": "Bash", "summary": "cargo test -p \(i)", "status": "Done", "started_ms": 1_000, "ended_ms": 2_000, "diff": [], "file_path": NSNull()]])
            }
            return try JSONSerialization.data(withJSONObject: ["epoch": 1, "rev": rev, "moreBefore": from > 0, "rows": rows])
        }

        func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data {
            try await Task.sleep(for: every)
            guard text.count < chars else {
                done = true
                try await Task.sleep(for: .seconds(3600))
                throw CancellationError()
            }
            rev += 1
            let words = "the quick brown fox jumps over a lazy dog while `code` and **bold** text flow by in a reply "
            let step = chars / 150
            var added = 0
            while added < step {
                text += words
                added += words.count
                if text.count % 900 < words.count { text += "\n\n" }
            }
            let kind = text.count == added ? "insert" : "update"
            let row = NativeAgentTests.row(held, "prose:reply", ["Prose": ["text": text, "conclusion": false, "at_ms": 10_000]])
            return try JSONSerialization.data(withJSONObject: ["epoch": 1, "rev": rev, "reset": false, "changes": [["kind": kind, "id": "prose:reply", "rev": rev, "row": row]]])
        }
    }

    @Test func aLongReplyStreamsIntoTheNativeViewWithoutStalling() async throws {
        let monitor = HangMonitor()
        monitor.start()
        defer { monitor.stop() }
        let source = Streaming(held: 5_000, chars: 25_600, every: .milliseconds(52))
        let store = AgentRowStore(key: "stall-\(UUID())", cache: nil)
        let model = NativePaneModel(terminal: "t", store: store, sink: nil)
        let window = NativeAgentTests.window(NativeAgentView(model: model, isFocused: true, showTerminal: {}))
        defer { window.close() }
        store.start(source)
        defer { store.stop() }

        let began = ContinuousClock.now
        while store.ids.isEmpty, ContinuousClock.now - began < .seconds(20) {
            try await Task.sleep(for: .milliseconds(5))
        }
        print("PERF[native] first page drawn after \(ContinuousClock.now - began)")
        _ = monitor.take()
        while await !source.done, ContinuousClock.now - began < .seconds(60) {
            try await Task.sleep(for: .milliseconds(50))
        }
        let when = monitor.when()
        let streaming = monitor.take()
        let longest = streaming.max() ?? 0
        print("PERF[native] 25.6K chars streamed: main thread: \(HangMonitor.describe(streaming)) [\(when)]")
        let times = store.applyTimes
        print(String(format: "PERF[native] apply: %d updates, max %.3f ms", times.count, times.map { Double($0.components.attoseconds) / 1e15 + Double($0.components.seconds) * 1000 }.max() ?? 0))
        #expect(longest <= 50, "a stall of \(longest) ms")
    }
}
