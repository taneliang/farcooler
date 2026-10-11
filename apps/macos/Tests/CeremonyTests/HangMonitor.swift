import Foundation

/// A main-thread hang detector the streaming timing harnesses share
/// (`NativeStreamingStallTests`, `NativeStreamingLiveTerminalTests`). It came
/// from the old chat's harness (ov-382), retired with chat mode (ov-377).
///
/// Pings the main queue from a background thread and records how late each
/// ping ran.
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
