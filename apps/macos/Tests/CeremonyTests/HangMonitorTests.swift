import Testing

/// `HangMonitor` splits a stall at `take`: each window holds the part of it
/// that fell inside (ov-703). Checked on `HangWindow` with exact times, in
/// ms here, since a real hold of the main thread can't be timed while the
/// suite's other tests use it: in the full run one measured 1,476 ms.
struct HangMonitorTests {
    private static func ns(_ ms: UInt64) -> UInt64 { 1_000_000_000 + ms * 1_000_000 }

    /// A 150 ms stall from a ping sent at 0, with `take` at 75: the half
    /// before is the closed window's, the half after the new one's.
    private func straddle() -> (closed: [Double], opened: [Double]) {
        var window = HangWindow(since: Self.ns(0))
        window.send(at: Self.ns(0))
        let closed = window.take(at: Self.ns(75))
        window.answer(sent: Self.ns(0), at: Self.ns(150))
        return (closed, window.take(at: Self.ns(200)))
    }

    /// A stall under way at the final `take` counts in the window it closes,
    /// up to that moment.
    @Test func aStallStraddlingTheEndCountsItsPartInside() {
        #expect(straddle().closed == [75])
    }

    /// A stall that runs on past a window's start counts there for its part
    /// after `take`: not all of it, nor none.
    @Test func aStallStraddlingTheStartCountsItsPartInside() {
        #expect(straddle().opened == [75])
    }

    /// The nightly's first run of `NativeStreamingStallTests` (Oct 25) failed
    /// on a 151 ms stall "at -150 ms": the first page's layout, which ended
    /// just after `take`, counted whole in the streaming window. Only its
    /// last millisecond is the new window's.
    @Test func aStallThatEndsJustAfterTakeIsTheClosedWindows() {
        var window = HangWindow(since: Self.ns(0))
        window.send(at: Self.ns(0))
        #expect(window.take(at: Self.ns(150)) == [150])
        window.answer(sent: Self.ns(0), at: Self.ns(151))
        #expect(window.take(at: Self.ns(200)) == [1])
    }

    /// A ping answered within its window counts whole, and `late` names a
    /// long one from where it began; one in flight is named as of now.
    @Test func aStallInsideAWindowCountsWhole() {
        var window = HangWindow(since: Self.ns(0))
        window.send(at: Self.ns(10))
        window.answer(sent: Self.ns(10), at: Self.ns(90))
        window.send(at: Self.ns(100))
        #expect(window.late(at: Self.ns(160)).map(\.at) == [10, 100])
        #expect(window.late(at: Self.ns(160)).map(\.ms) == [80, 60])
        #expect(window.take(at: Self.ns(160)) == [80, 60])
    }
}
