import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// ov-382: the real `AgentSurface`, in a real offscreen window, fed by a
/// stand-in CLI that prints what `agent-subscribe --follow` prints, while a
/// long reply streams into its last row. Measures what the reader feels: how
/// late the main thread answers while the reply streams, how long a scroll
/// frame takes, and the one redraw when the turn ends.
///
/// Opt-in, as a timing harness on a shared machine can't be a gate: runs only
/// with FARCOOLER_PERF=1 and FC_FX naming a fixture
/// (`scripts/perf/streaming-reply-fixture.py`). The gate is
/// `StreamingReplyPerfTests` in AgentKit.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1"))
struct StreamingReplyStallTests {
    static let env = ProcessInfo.processInfo.environment
    static let fakeCLI = env["FC_BIN"] ?? "scripts/perf/fake-agent-follow.py"

    /// Pings the main queue from a background thread and records how late
    /// each ping ran: a main-thread hang detector.
    final class HangMonitor: @unchecked Sendable {
        private let lock = NSLock()
        private var stalls: [Double] = []
        /// When each stall over 50 ms began, in ms since `take` last ran.
        private var late: [(at: Double, ms: Double)] = []
        private var since = DispatchTime.now().uptimeNanoseconds
        private var running = true

        func start() {
            Thread { [self] in
                while lock.withLock({ running }) {
                    let sent = DispatchTime.now().uptimeNanoseconds
                    let done = DispatchSemaphore(value: 0)
                    DispatchQueue.main.async { done.signal() }
                    done.wait()
                    let late = Double(DispatchTime.now().uptimeNanoseconds - sent) / 1e6
                    lock.withLock {
                        stalls.append(late)
                        if late > 50 { self.late.append((Double(Int64(bitPattern: sent &- since)) / 1e6, late)) }
                    }
                    Thread.sleep(forTimeInterval: 0.004)
                }
            }.start()
        }

        func take() -> [Double] {
            lock.withLock {
                let s = stalls
                stalls = []
                late = []
                since = DispatchTime.now().uptimeNanoseconds
                return s
            }
        }

        /// The stalls over 50 ms since `take`, as "120 ms at 3400 ms".
        func when() -> String {
            lock.withLock { late.map { String(format: "%.0f ms at %.0f ms", $0.ms, $0.at) }.joined(separator: ", ") }
        }
        func stop() { lock.withLock { running = false } }

        static func describe(_ s: [Double]) -> String {
            let sorted = s.sorted()
            guard let worst = sorted.last else { return "no samples" }
            let over50 = s.filter { $0 > 50 }
            return String(
                format: "max %.0f ms, p99 %.0f ms, stalls>50ms %d (sum %.0f ms)",
                worst, sorted[min(sorted.count - 1, Int(Double(sorted.count) * 0.99))],
                over50.count, over50.reduce(0, +))
        }
    }

    @MainActor
    final class Mount: ObservableObject {
        @Published var terminal: Terminal
        init(_ terminal: Terminal) { self.terminal = terminal }
    }

    struct Hosted: View {
        @ObservedObject var mount: Mount
        let environment: [String: String]
        var body: some View {
            AgentSurface(
                terminal: mount.terminal, binary: StreamingReplyStallTests.fakeCLI, environment: environment,
                hostArguments: [], linkGeneration: 0, refusal: { nil }, isFocused: true,
                searchFiles: { _ in [] }, onResize: { _, _ in }
            )
            .frame(width: 900, height: 760)
        }
    }

    static func terminal(activity: String) throws -> Terminal {
        let json = #"""
            {"id":"t1","short":"t1","title":"Terminal 1","preset":"claude","state":"running",
             "epoch":1,"paneMode":"agent","activity":"\#(activity)"}
            """#
        return try JSONDecoder().decode(Terminal.self, from: Data(json.utf8))
    }

    static func now() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }

    /// The transcript's scroll view: the one with the tallest document.
    static func scrollView(in view: NSView) -> NSScrollView? {
        var best: NSScrollView?
        func walk(_ v: NSView) {
            if let s = v as? NSScrollView,
                (s.documentView?.frame.height ?? 0) > (best?.documentView?.frame.height ?? 0)
            {
                best = s
            }
            v.subviews.forEach(walk)
        }
        walk(view)
        return best
    }

    static func p95(_ times: [Double]) -> Double {
        let sorted = times.sorted()
        return sorted.isEmpty ? 0 : sorted[min(sorted.count - 1, Int(Double(sorted.count) * 0.95))]
    }

    /// Steps of `by` points from where it is to `to`, one displayed frame
    /// each; the frame times.
    static func scroll(_ s: NSScrollView, window: NSWindow, to target: CGFloat, by step: CGFloat) async -> [Double] {
        var times: [Double] = []
        var y = s.contentView.bounds.origin.y
        while abs(target - y) > 1, times.count < 3000 {
            y = target > y ? min(target, y + step) : max(target, y - step)
            let a = now()
            s.contentView.scroll(to: NSPoint(x: 0, y: y))
            s.reflectScrolledClipView(s.contentView)
            window.displayIfNeeded()
            times.append(now() - a)
            try? await Task.sleep(for: .milliseconds(8))
        }
        return times
    }

    @Test func aLongReplyStreamsWithoutStalling() async throws {
        let fixture = Self.env["FC_FX"] ?? "?"
        // For a profiler: `sample <pid>` while it streams.
        print("PERF[\(fixture)] pid \(getpid())")
        var environment = Self.env
        environment["FC_FX_STREAM"] = Self.env["FC_FX_STREAM"] ?? "52"
        let monitor = HangMonitor()
        monitor.start()
        defer { monitor.stop() }

        let mount = Mount(try Self.terminal(activity: Self.env["FC_ACTIVITY"] ?? "working"))
        let host = NSHostingView(rootView: Hosted(mount: mount, environment: environment))
        let window = NSWindow(
            contentRect: NSRect(x: -7000, y: -7000, width: 900, height: 760), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.close() }

        // The fixture arrives and lays out.
        let t0 = Self.now()
        while Self.now() - t0 < 30_000,
            (Self.scrollView(in: host)?.documentView?.frame.height ?? 0) < 900
        {
            try? await Task.sleep(for: .milliseconds(2))
        }
        print(String(format: "PERF[%@] first content after %.0f ms", fixture, Self.now() - t0))

        // The rest of the first layout, which the stream is already
        // arriving into: a mount's cost, not a delta's.
        _ = monitor.take()
        try? await Task.sleep(for: .milliseconds(1000))
        print("PERF[\(fixture)] the second after first content: main thread: \(HangMonitor.describe(monitor.take()))")

        // 8 s of streaming at the tail, the reader following it.
        try? await Task.sleep(for: .milliseconds(Int(Self.env["FC_FX_STREAM_MS"] ?? "8000") ?? 8000))
        let when = monitor.when()
        let streaming = monitor.take()
        print("PERF[\(fixture)] 8 s of streaming at the tail: main thread: \(HangMonitor.describe(streaming)) [\(when)]")

        // The turn ends: the reply is drawn settled, selectable across its
        // paragraphs.
        try? await Task.sleep(for: .milliseconds(1500))
        _ = monitor.take()
        mount.terminal = try Self.terminal(activity: "idle")
        try? await Task.sleep(for: .milliseconds(1500))
        print("PERF[\(fixture)] the turn ends: main thread: \(HangMonitor.describe(monitor.take()))")

        // Scrolling the settled transcript up and back down.
        if let s = Self.scrollView(in: host), let doc = s.documentView {
            _ = monitor.take()
            let up = await Self.scroll(s, window: window, to: 0, by: 100)
            let down = await Self.scroll(
                s, window: window, to: doc.frame.height - s.contentView.bounds.height, by: 100)
            print(String(
                format: "PERF[%@] scroll over %d pt: up p95 %.1f ms max %.0f ms; down p95 %.1f ms max %.0f ms; main thread: %@",
                fixture, Int(doc.frame.height), Self.p95(up), up.max() ?? 0, Self.p95(down), down.max() ?? 0,
                HangMonitor.describe(monitor.take())))
        }
    }
}
