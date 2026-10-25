import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// ov-372's streaming check, by ov-382's method: the real `NativeAgentView`
/// in a real offscreen window, a session of 5,000 rows held, and a reply
/// growing to 25.6K characters in its newest prose row, one follow every
/// 52 ms, through the production store and loop. A thread pings the main
/// queue and records how late each ping ran (`HangMonitor`); the budget is no stall over 50 ms while it streams.
///
/// And the first page of that session coming in (ov-750): no stall over
/// 50 ms from the store's start until its rows are laid out, with only the
/// rows on screen built and the reader at the tail.
///
/// Opt-in, as a timing harness on a shared machine can't be a gate: runs
/// only with FARCOOLER_PERF=1.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1"))
struct NativeStreamingStallTests {

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

    /// The native view of `source`'s session in an offscreen window, made
    /// and drawn empty first, then the store started and its first page
    /// laid out. Returns the main thread's waits from the store's start
    /// until that layout is done: the first page's window, with the
    /// window's own making outside it.
    ///
    /// Until ov-750 the first page's window opened when `HangMonitor` did,
    /// before the window was made. Making it (`NSWindow`, the hosting view,
    /// the composer's text view) holds the main thread 43 to 54 ms on an
    /// M-series Mac the first time in a process, about a third of it in
    /// `NSWindow`'s own init, and the first ping waited through that and
    /// the page together: the 53 to 280 ms printed as the first page's
    /// (ov-703). Laid out on its own, the page is 11 to 42 ms, nearly all
    /// of it the lazy stack sizing its rows.
    static func open(_ source: Streaming, monitor: HangMonitor) async throws
        -> (window: NSWindow, model: NativePaneModel, scroll: NSScrollView, firstPage: [Double])
    {
        let store = AgentRowStore(key: "stall-\(UUID())", cache: nil)
        let model = NativePaneModel(terminal: "t", store: store, sink: nil)
        let window = NativeAgentTests.window(NativeAgentView(model: model, isFocused: true, showTerminal: {}))
        await NativeAgentTests.settle(window, 200)
        print("PERF[native] window made and drawn empty: main thread: \(HangMonitor.describe(monitor.take())) (not gated)")

        store.start(source)
        let began = ContinuousClock.now
        while store.ids.isEmpty, ContinuousClock.now - began < .seconds(120) {
            try await Task.sleep(for: .milliseconds(5))
        }
        // Whatever layout of the page is still pending runs now, inside the
        // window, rather than after it closes.
        window.contentView?.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let firstPage = monitor.take()
        print("PERF[native] first page laid out after \(ContinuousClock.now - began)")
        let scroll = try #require(window.contentView.flatMap(Fling.scrollView(in:)), "no scroll view")
        return (window, model, scroll, firstPage)
    }

    /// 50 ms here; 100 on the nightly's runner (`PerfBudget`), whose main
    /// thread already ran 70 ms frames in a fling on Oct 22.
    static let budget = PerfBudget.limit(local: 50, runner: 100)

    /// The first page of a 5,000-row session (`AgentRowStore.pageSize`
    /// rows) lays out without stalling: only the rows on screen are built,
    /// the reader is at the tail, and the main thread is never held over
    /// the budget. A stack that built and laid out every row of the page
    /// at once fails all three (ov-750).
    @Test func theFirstPageOfALongSessionLaysOutWithoutStalling() async throws {
        let monitor = HangMonitor()
        monitor.start()
        defer { monitor.stop() }
        // No reply: the first page alone.
        let source = Streaming(held: 5_000, chars: 0, every: .milliseconds(52))
        let (window, model, scroll, firstPage) = try await Self.open(source, monitor: monitor)
        defer { window.close() }
        defer { model.store.stop() }

        let store = model.store
        let layout = try #require(TranscriptScroller.driving(scroll)?.layout, "the transcript's scroller isn't driven")
        let built = store.items.filter { layout.has($0.id) }.count
        print("PERF[native] first page: \(store.ids.count) rows, \(store.items.count) items, \(built) built")
        #expect(store.ids.count == AgentRowStore.pageSize, "the first page is \(store.ids.count) rows")
        // 12 of 50 are built in a 560 pt window: those on screen and the
        // lazy stack's margin around them.
        #expect(built > 0 && built * 2 < store.items.count, "the first page built \(built) of \(store.items.count) items")
        let insets = scroll.contentInsets
        let range = (scroll.documentView?.frame.height ?? 0) + insets.top + insets.bottom - scroll.contentView.bounds.height
        let y = scroll.contentView.bounds.minY + insets.top
        #expect(range > 0 && abs(y - range) < 1, "the reader is at \(y) of \(range), not the tail")

        let longest = firstPage.max() ?? 0
        print("PERF[native] first page: main thread: \(HangMonitor.describe(firstPage)); " + PerfBudget.describe(Self.budget, unit: "ms"))
        #expect(Double(longest) <= Self.budget, "the first page held the main thread \(longest) ms")
    }

    @Test func aLongReplyStreamsIntoTheNativeViewWithoutStalling() async throws {
        let monitor = HangMonitor()
        monitor.start()
        defer { monitor.stop() }
        let source = Streaming(held: 5_000, chars: 25_600, every: .milliseconds(52))
        let (window, model, _, firstPage) = try await Self.open(source, monitor: monitor)
        defer { window.close() }
        defer { model.store.stop() }
        let store = model.store
        // The first page's stalls are its own test's to gate.
        print("PERF[native] first page: main thread: \(HangMonitor.describe(firstPage)) (gated by the first page's test)")

        let began = ContinuousClock.now
        while await !source.done, ContinuousClock.now - began < .seconds(60) {
            try await Task.sleep(for: .milliseconds(50))
        }
        let when = monitor.when()
        let streaming = monitor.take()
        let longest = streaming.max() ?? 0
        print("PERF[native] 25.6K chars streamed: main thread: \(HangMonitor.describe(streaming)) [\(when)]")
        let times = store.applyTimes
        print(String(format: "PERF[native] apply: %d updates, max %.3f ms", times.count, times.map { Double($0.components.attoseconds) / 1e15 + Double($0.components.seconds) * 1000 }.max() ?? 0))
        // Its first night on the runner (Oct 25), streaming's p99 was 28 ms.
        print("PERF[native] longest stall \(longest) ms; " + PerfBudget.describe(Self.budget, unit: "ms"))
        #expect(Double(longest) <= Self.budget, "a stall of \(longest) ms")
    }
}
