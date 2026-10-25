import Foundation
import Testing

/// `HangMonitor` splits a stall at `take`: each window holds the part of it
/// that fell inside (ov-703).
/// Serialized: each test holds the one main thread the others measure.
@MainActor
@Suite(.serialized)
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

    /// A stall under way when `take` runs from another thread, 75 ms into a
    /// 150 ms hold of the main thread: the half before is the closed window's
    /// (the ping still in flight), and the half after is the new one's.
    private func straddle() async throws -> (closed: Double, opened: Double) {
        let monitor = HangMonitor()
        monitor.start()
        defer { monitor.stop() }
        try await Task.sleep(for: .milliseconds(30))
        _ = monitor.take()
        let closed = Closed()
        DispatchQueue.global().asyncAfter(deadline: .now() + .milliseconds(75)) { closed.set(monitor.take()) }
        let held = ContinuousClock.now + .milliseconds(150)
        while ContinuousClock.now < held {}
        try await Task.sleep(for: .milliseconds(60))
        let opened = monitor.take()
        return (closed.get().max() ?? 0, opened.max() ?? 0)
    }

    final class Closed: @unchecked Sendable {
        private let lock = NSLock()
        private var samples: [Double] = []
        func set(_ s: [Double]) { lock.withLock { samples = s } }
        func get() -> [Double] { lock.withLock { samples } }
    }

    /// A stall that runs on past the window's start counts there for its part
    /// after `take`, not all of it, nor none.
    @Test func aStallStraddlingTheStartCountsItsPartInside() async throws {
        let opened = try await straddle().opened
        #expect(opened > 40 && opened < 110, "the new window's part of a 150 ms stall split at 75 ms: \(opened) ms")
    }

    /// A stall under way at the final `take` counts in the window it closes,
    /// up to that moment.
    @Test func aStallStraddlingTheEndCountsItsPartInside() async throws {
        let closed = try await straddle().closed
        #expect(closed > 40 && closed < 110, "the closed window's part of a 150 ms stall split at 75 ms: \(closed) ms")
    }
}
