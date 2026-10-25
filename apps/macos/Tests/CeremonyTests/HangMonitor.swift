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
    private var window = HangWindow(since: DispatchTime.now().uptimeNanoseconds)
    private var running = true

    func start() {
        Thread { [self] in
            while lock.withLock({ running }) {
                let sent = DispatchTime.now().uptimeNanoseconds
                lock.withLock { window.send(at: sent) }
                let done = DispatchSemaphore(value: 0)
                DispatchQueue.main.async { done.signal() }
                done.wait()
                lock.withLock { window.answer(sent: sent, at: DispatchTime.now().uptimeNanoseconds) }
                Thread.sleep(forTimeInterval: 0.004)
            }
        }.start()
    }

    func take() -> [Double] {
        lock.withLock { window.take(at: DispatchTime.now().uptimeNanoseconds) }
    }

    /// The stalls over 50 ms since `take`, as "120 ms at 3400 ms", the one
    /// under way included.
    func when() -> String {
        lock.withLock {
            window.late(at: DispatchTime.now().uptimeNanoseconds)
                .map { String(format: "%.0f ms at %.0f ms", $0.ms, $0.at) }.joined(separator: ", ")
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

/// `HangMonitor`'s windows, as arithmetic on the times it is handed (uptime
/// nanoseconds), so its edges can be checked with exact times and no main
/// thread (`HangMonitorTests`).
struct HangWindow {
    /// When this window opened: the last `take`.
    private(set) var since: UInt64
    /// Each ping's wait inside this window, in ms.
    private(set) var stalls: [Double] = []
    /// Each of those over 50 ms: where it began, in ms since `since` (0 for
    /// one that began before), and its part here.
    private var long: [(at: Double, ms: Double)] = []
    /// When the ping still waiting on the main queue was sent.
    private var inFlight: UInt64?

    init(since: UInt64) { self.since = since }

    mutating func send(at sent: UInt64) { inFlight = sent }

    /// A ping sent at `sent` ran at `at`: its wait since this window opened.
    mutating func answer(sent: UInt64, at: UInt64) {
        inFlight = nil
        count(sent: sent, until: at)
    }

    /// Close this window at `at`, the ping in flight counted up to then,
    /// and open the next. Returns the closed window's waits.
    mutating func take(at: UInt64) -> [Double] {
        if let sent = inFlight { count(sent: sent, until: at) }
        let closed = stalls
        stalls = []
        long = []
        since = at
        return closed
    }

    /// The waits over 50 ms in this window so far, the one in flight
    /// included as of `at`.
    func late(at: UInt64) -> [(at: Double, ms: Double)] {
        guard let sent = inFlight, let part = part(sent: sent, until: at), part.ms > 50 else { return long }
        return long + [part]
    }

    private func part(sent: UInt64, until: UInt64) -> (at: Double, ms: Double)? {
        let from = max(sent, since)
        guard until > from else { return nil }
        return (Double(from - since) / 1e6, Double(until - from) / 1e6)
    }

    private mutating func count(sent: UInt64, until: UInt64) {
        guard let part = part(sent: sent, until: until) else { return }
        stalls.append(part.ms)
        if part.ms > 50 { long.append(part) }
    }
}
