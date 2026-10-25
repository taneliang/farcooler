import Foundation
import Testing

/// `HangMonitor` counts a stall in the window it began in (ov-703).
@MainActor
struct HangMonitorTests {

    /// The nightly's first run of `NativeStreamingStallTests` failed on a
    /// 151 ms stall "at -150 ms": a ping sent while the first page laid out,
    /// answered just after `take` opened the streaming window, and counted
    /// in it. A stall that ends inside a window but began before it is the
    /// window before's.
    @Test func aStallThatBeganBeforeTakeIsNotCountedAfterIt() async throws {
        let monitor = HangMonitor()
        monitor.start()
        defer { monitor.stop() }
        try await Task.sleep(for: .milliseconds(30))
        // The main thread held 150 ms, then the window opened in the same
        // turn, before the ping queued behind the hold could run.
        let held = ContinuousClock.now + .milliseconds(150)
        while ContinuousClock.now < held {}
        _ = monitor.take()
        try await Task.sleep(for: .milliseconds(60))
        let after = monitor.take()
        #expect(!after.isEmpty, "the monitor stopped pinging")
        #expect((after.max() ?? 0) < 100, "the stall before take counted after it: \(after.max() ?? 0) ms")
    }
}
