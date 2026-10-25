import Foundation

/// A main-thread hang detector the streaming timing harnesses share
/// (`NativeStreamingStallTests`, `NativeStreamingLiveTerminalTests`). It came
/// from the old chat's harness (ov-382), retired with chat mode (ov-377).
///
/// Pings the main queue from a background thread and records how late each
/// ping ran. A window (from one `take` to the next) holds the part of each
/// stall that fell inside it: a stall that straddles `take` is split there,
/// the part before it in the window `take` closed (a ping still in flight is
/// measured up to that moment) and the rest in the one it opened. Counted
/// whole in the new window, the first page's layout once read as a 151 ms
/// stall while a reply streamed; dropped, a stall running on into a window,
/// or into the final `take`, went unseen (ov-703).
final class HangMonitor: @unchecked Sendable {
    private let lock = NSLock()
    private var stalls: [Double] = []
    /// Each stall over 50 ms in this window: where it began, in ms since
    /// `take` last ran (0 for one that began before), and its part here.
    private var late: [(at: Double, ms: Double)] = []
    private var since = DispatchTime.now().uptimeNanoseconds
    /// When the ping still waiting on the main queue was sent.
    private var inFlight: UInt64?
    private var running = true

    func start() {
        Thread { [self] in
            while lock.withLock({ running }) {
                let sent = DispatchTime.now().uptimeNanoseconds
                lock.withLock { inFlight = sent }
                let done = DispatchSemaphore(value: 0)
                DispatchQueue.main.async { done.signal() }
                done.wait()
                let now = DispatchTime.now().uptimeNanoseconds
                lock.withLock {
                    inFlight = nil
                    record(sent: sent, until: now)
                }
                Thread.sleep(forTimeInterval: 0.004)
            }
        }.start()
    }

    /// The part of a ping's wait from `sent` to `until` inside this window.
    /// Called under the lock.
    private func record(sent: UInt64, until: UInt64) {
        let from = max(sent, since)
        guard until > from else { return }
        let ms = Double(until - from) / 1e6
        stalls.append(ms)
        if ms > 50 { late.append((Double(from - since) / 1e6, ms)) }
    }

    func take() -> [Double] {
        lock.withLock {
            let now = DispatchTime.now().uptimeNanoseconds
            // The ping still waiting has waited this long in this window.
            if let sent = inFlight { record(sent: sent, until: now) }
            let s = stalls
            stalls = []
            late = []
            since = now
            return s
        }
    }

    /// The stalls over 50 ms since `take`, as "120 ms at 3400 ms", the one
    /// under way included.
    func when() -> String {
        lock.withLock {
            var all = late
            if let sent = inFlight {
                let from = max(sent, since)
                let now = DispatchTime.now().uptimeNanoseconds
                let ms = now > from ? Double(now - from) / 1e6 : 0
                if ms > 50 { all.append((Double(from - since) / 1e6, ms)) }
            }
            return all.map { String(format: "%.0f ms at %.0f ms", $0.ms, $0.at) }.joined(separator: ", ")
        }
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
